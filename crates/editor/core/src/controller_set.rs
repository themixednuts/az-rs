//! Attached editor-controller lifecycle.
//!
//! This module is the sole owner of controllers whose lifetime is tied to a
//! verified [`EditorAttachSession`]. Domain modules own their RPCs and typed
//! projections; this set owns installation, retries, fencing, and cancellation
//! by retaining each controller in one typed slot.

use gpui::{App, Global};
use thiserror::Error;
use tracing::error;

use crate::EditorAttachSession;
use crate::asset_processor::{
    AssetBrowserSnapshotRefreshRequest, AssetProcessorEventAdmission,
    AssetProcessorEventStreamCursor, AssetProcessorEventStreamToken,
    AssetProcessorEventSubscription, AssetProcessorSnapshotAdmission, EditorAssetBrowserController,
    EditorAssetBrowserSnapshotRefreshState, EditorAssetProcessorEventStreamOwner,
    PendingAssetBrowserSnapshotRefresh,
};
use crate::authored_selection::EditorReflectedSelectionController;
use crate::game_data_catalog::EditorGameDataCatalogController;
use crate::graph_ui::{
    EditorGraphController, GraphActionAdmission, GraphActionQueue, GraphActionQueueIdentity,
    GraphControllerAction,
};
use crate::mannequin_animation::EditorMannequinAnimationController;
use crate::project_build::EditorProjectBuildController;
use crate::recovery::EditorRecoveryController;
use crate::runtime_host::EditorRuntimeController;
use crate::sequencer::EditorSequencerController;
use crate::session_supervisor::EditorSessionStatusController;
use crate::{EditorError, EditorResult};

const CONTROLLER_KIND_COUNT: usize = 10;

/// The closed set of controllers attached to one verified editor session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ControllerKind {
    ReflectedSelection,
    GameData,
    AssetBrowser,
    Graph,
    MannequinAnimation,
    Sequencer,
    Recovery,
    SessionStatus,
    ProjectBuild,
    Runtime,
}

impl ControllerKind {
    pub(crate) const ALL: [Self; CONTROLLER_KIND_COUNT] = [
        Self::ReflectedSelection,
        Self::GameData,
        Self::AssetBrowser,
        Self::Graph,
        Self::MannequinAnimation,
        Self::Sequencer,
        Self::Recovery,
        Self::SessionStatus,
        Self::ProjectBuild,
        Self::Runtime,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn expected_policy(self) -> ControllerPolicy {
        match self {
            Self::Runtime => ControllerPolicy::Optional,
            Self::ReflectedSelection
            | Self::GameData
            | Self::AssetBrowser
            | Self::Graph
            | Self::MannequinAnimation
            | Self::Sequencer
            | Self::Recovery
            | Self::SessionStatus
            | Self::ProjectBuild => ControllerPolicy::Required,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::ReflectedSelection => "reflected selection",
            Self::GameData => "GameData",
            Self::AssetBrowser => "asset browser",
            Self::Graph => "graph",
            Self::MannequinAnimation => "mannequin animation",
            Self::Sequencer => "sequencer",
            Self::Recovery => "recovery",
            Self::SessionStatus => "session status",
            Self::ProjectBuild => "project build",
            Self::Runtime => "runtime",
        }
    }

    const fn action_kind(self) -> az_editor_ui::actions::AttachedControllerKind {
        match self {
            Self::ReflectedSelection => {
                az_editor_ui::actions::AttachedControllerKind::ReflectedSelection
            }
            Self::GameData => az_editor_ui::actions::AttachedControllerKind::GameData,
            Self::AssetBrowser => az_editor_ui::actions::AttachedControllerKind::AssetBrowser,
            Self::Graph => az_editor_ui::actions::AttachedControllerKind::Graph,
            Self::MannequinAnimation => {
                az_editor_ui::actions::AttachedControllerKind::MannequinAnimation
            }
            Self::Sequencer => az_editor_ui::actions::AttachedControllerKind::Sequencer,
            Self::Recovery => az_editor_ui::actions::AttachedControllerKind::Recovery,
            Self::SessionStatus => az_editor_ui::actions::AttachedControllerKind::SessionStatus,
            Self::ProjectBuild => az_editor_ui::actions::AttachedControllerKind::ProjectBuild,
            Self::Runtime => az_editor_ui::actions::AttachedControllerKind::Runtime,
        }
    }
}

impl From<az_editor_ui::actions::AttachedControllerKind> for ControllerKind {
    fn from(kind: az_editor_ui::actions::AttachedControllerKind) -> Self {
        match kind {
            az_editor_ui::actions::AttachedControllerKind::ReflectedSelection => {
                Self::ReflectedSelection
            }
            az_editor_ui::actions::AttachedControllerKind::GameData => Self::GameData,
            az_editor_ui::actions::AttachedControllerKind::AssetBrowser => Self::AssetBrowser,
            az_editor_ui::actions::AttachedControllerKind::Graph => Self::Graph,
            az_editor_ui::actions::AttachedControllerKind::MannequinAnimation => {
                Self::MannequinAnimation
            }
            az_editor_ui::actions::AttachedControllerKind::Sequencer => Self::Sequencer,
            az_editor_ui::actions::AttachedControllerKind::Recovery => Self::Recovery,
            az_editor_ui::actions::AttachedControllerKind::SessionStatus => Self::SessionStatus,
            az_editor_ui::actions::AttachedControllerKind::ProjectBuild => Self::ProjectBuild,
            az_editor_ui::actions::AttachedControllerKind::Runtime => Self::Runtime,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControllerPolicy {
    Required,
    Optional,
}

type ControllerInstaller = fn(&mut App, EditorAttachSession, ControllerFence);

#[derive(Clone, Copy)]
struct ControllerDescriptor {
    kind: ControllerKind,
    policy: ControllerPolicy,
    install: ControllerInstaller,
}

const CONTROLLER_INSTALL_PLAN: [ControllerDescriptor; CONTROLLER_KIND_COUNT] = [
    ControllerDescriptor {
        kind: ControllerKind::ReflectedSelection,
        policy: ControllerPolicy::Required,
        install: crate::authored_selection::install_reflected_selection_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::GameData,
        policy: ControllerPolicy::Required,
        install: crate::game_data_catalog::install_game_data_catalog_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::AssetBrowser,
        policy: ControllerPolicy::Required,
        install: crate::asset_processor::install_asset_browser_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::Graph,
        policy: ControllerPolicy::Required,
        install: crate::graph_ui::install_graph_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::MannequinAnimation,
        policy: ControllerPolicy::Required,
        install: crate::mannequin_animation::install_mannequin_animation_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::Sequencer,
        policy: ControllerPolicy::Required,
        install: crate::sequencer::install_sequencer_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::Recovery,
        policy: ControllerPolicy::Required,
        install: crate::recovery::install_recovery_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::SessionStatus,
        policy: ControllerPolicy::Required,
        install: crate::session_supervisor::install_session_status_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::ProjectBuild,
        policy: ControllerPolicy::Required,
        install: crate::project_build::install_project_build_slot,
    },
    ControllerDescriptor {
        kind: ControllerKind::Runtime,
        policy: ControllerPolicy::Optional,
        install: crate::runtime_host::install_runtime_slot,
    },
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControllerInstallPlanError {
    #[error("controller install plan is missing `{kind}`")]
    Missing { kind: &'static str },
    #[error("controller install plan registers `{kind}` more than once")]
    Duplicate { kind: &'static str },
    #[error("controller install plan gives `{kind}` the wrong required/optional policy")]
    PolicyMismatch { kind: &'static str },
}

fn validate_controller_install_plan(
    descriptors: &[ControllerDescriptor],
) -> Result<(), ControllerInstallPlanError> {
    let mut seen = [false; CONTROLLER_KIND_COUNT];
    for descriptor in descriptors {
        let index = descriptor.kind.index();
        if seen[index] {
            return Err(ControllerInstallPlanError::Duplicate {
                kind: descriptor.kind.name(),
            });
        }
        if descriptor.policy != descriptor.kind.expected_policy() {
            return Err(ControllerInstallPlanError::PolicyMismatch {
                kind: descriptor.kind.name(),
            });
        }
        seen[index] = true;
    }
    for kind in ControllerKind::ALL {
        if !seen[kind.index()] {
            return Err(ControllerInstallPlanError::Missing { kind: kind.name() });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerFence {
    kind: ControllerKind,
    generation: u64,
    attempt: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControllerLifecycle {
    generation: u64,
    attempts: [u64; CONTROLLER_KIND_COUNT],
}

impl ControllerLifecycle {
    const fn first() -> Self {
        Self {
            generation: 1,
            attempts: [1; CONTROLLER_KIND_COUNT],
        }
    }

    const fn after(previous: Self) -> Self {
        let generation = previous.generation.wrapping_add(1);
        Self {
            generation: if generation == 0 { 1 } else { generation },
            attempts: [1; CONTROLLER_KIND_COUNT],
        }
    }

    const fn fence(self, kind: ControllerKind) -> ControllerFence {
        ControllerFence {
            kind,
            generation: self.generation,
            attempt: self.attempts[kind.index()],
        }
    }

    const fn is_current(self, fence: ControllerFence) -> bool {
        self.generation == fence.generation && self.attempts[fence.kind.index()] == fence.attempt
    }

    const fn retry(&mut self, kind: ControllerKind) -> ControllerFence {
        let attempt = self.attempts[kind.index()].wrapping_add(1);
        self.attempts[kind.index()] = if attempt == 0 { 1 } else { attempt };
        self.fence(kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorControllerFailure {
    message: String,
}

/// The central status surface derives this directly from the lifecycle owner.
/// It is intentionally presentation data, not another health global.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerFailurePresentation {
    pub(crate) kind: ControllerKind,
    pub(crate) message: String,
    pub(crate) retry: az_editor_ui::actions::RetryAttachedController,
}

impl EditorControllerFailure {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

/// One typed controller slot. Missing wiring is never represented as empty
/// domain data: every attached domain is installing, ready, unavailable by
/// declared optional policy, or explicitly failed.
pub enum ControllerSlot<T> {
    Installing,
    Ready(T),
    Unavailable,
    Failed(EditorControllerFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerSlotState {
    Installing,
    Ready,
    Unavailable,
    Failed,
}

impl<T> ControllerSlot<T> {
    const fn state(&self) -> ControllerSlotState {
        match self {
            Self::Installing => ControllerSlotState::Installing,
            Self::Ready(_) => ControllerSlotState::Ready,
            Self::Unavailable => ControllerSlotState::Unavailable,
            Self::Failed(_) => ControllerSlotState::Failed,
        }
    }

    const fn failure(&self) -> Option<&EditorControllerFailure> {
        match self {
            Self::Failed(failure) => Some(failure),
            Self::Installing | Self::Ready(_) | Self::Unavailable => None,
        }
    }
}

/// Applies an injected completion only when the slot still belongs to its
/// original generation/attempt. Keeping this pure makes lifecycle tests
/// deterministic even though `spawn_editor_rpc` completes inline under tests.
fn complete_if_current<T>(
    lifecycle: ControllerLifecycle,
    fence: ControllerFence,
    slot: &mut ControllerSlot<T>,
    controller: T,
) -> bool {
    if !lifecycle.is_current(fence) || !matches!(slot, ControllerSlot::Installing) {
        return false;
    }
    *slot = ControllerSlot::Ready(controller);
    true
}

/// Applies an injected failure only while the intended slot is still
/// installing for this generation/attempt. This is deliberately separate
/// from descriptor-plan validation: a bad plan prevents publication, whereas
/// a runtime installer error becomes a visible slot state.
fn fail_if_current<T>(
    lifecycle: ControllerLifecycle,
    fence: ControllerFence,
    slot: &mut ControllerSlot<T>,
    message: impl Into<String>,
) -> bool {
    if !lifecycle.is_current(fence) || !matches!(slot, ControllerSlot::Installing) {
        return false;
    }
    *slot = ControllerSlot::Failed(EditorControllerFailure::new(message));
    true
}

const fn initial_runtime_slot<T>(runtime_endpoint_present: bool) -> ControllerSlot<T> {
    if runtime_endpoint_present {
        ControllerSlot::Installing
    } else {
        ControllerSlot::Unavailable
    }
}

/// A controller handle copied from the aggregate with the fence that owns it.
/// Any asynchronous completion based on this value must publish through the
/// matching fence-aware operation below.
#[derive(Clone)]
pub struct AttachedController<T> {
    pub(crate) fence: ControllerFence,
    pub(crate) controller: T,
}

/// The one GPUI global that owns all session-attached controller lifecycles.
///
/// It intentionally does not own mode projections, domain catalogs, or UI
/// state. Those are typed observations produced by the controllers and remain
/// independently owned by their domain/mode modules.
pub struct EditorControllers {
    session: EditorAttachSession,
    lifecycle: ControllerLifecycle,
    reflected_selection: ControllerSlot<EditorReflectedSelectionController>,
    game_data: ControllerSlot<EditorGameDataCatalogController>,
    asset_browser: ControllerSlot<EditorAssetBrowserController>,
    graph: ControllerSlot<EditorGraphController>,
    mannequin_animation: ControllerSlot<EditorMannequinAnimationController>,
    sequencer: ControllerSlot<EditorSequencerController>,
    recovery: ControllerSlot<EditorRecoveryController>,
    session_status: ControllerSlot<EditorSessionStatusController>,
    project_build: ControllerSlot<EditorProjectBuildController>,
    runtime: ControllerSlot<EditorRuntimeController>,
    asset_processor_stream: EditorAssetProcessorEventStreamOwner,
    asset_browser_snapshot_refresh: EditorAssetBrowserSnapshotRefreshState,
    graph_actions: GraphActionQueue,
}

impl Global for EditorControllers {}

impl EditorControllers {
    fn installing(session: EditorAttachSession, lifecycle: ControllerLifecycle) -> Self {
        let runtime = initial_runtime_slot(session.services.runtime_host.is_some());
        Self {
            session,
            lifecycle,
            reflected_selection: ControllerSlot::Installing,
            game_data: ControllerSlot::Installing,
            asset_browser: ControllerSlot::Installing,
            graph: ControllerSlot::Installing,
            mannequin_animation: ControllerSlot::Installing,
            sequencer: ControllerSlot::Installing,
            recovery: ControllerSlot::Installing,
            session_status: ControllerSlot::Installing,
            project_build: ControllerSlot::Installing,
            runtime,
            asset_processor_stream: EditorAssetProcessorEventStreamOwner::default(),
            asset_browser_snapshot_refresh: EditorAssetBrowserSnapshotRefreshState::default(),
            graph_actions: GraphActionQueue::default(),
        }
    }

    const fn current_fence(&self, kind: ControllerKind) -> ControllerFence {
        self.lifecycle.fence(kind)
    }

    const fn is_current(&self, fence: ControllerFence) -> bool {
        self.lifecycle.is_current(fence)
    }

    fn complete<T>(
        &mut self,
        fence: ControllerFence,
        controller: T,
        slot: impl FnOnce(&mut Self) -> &mut ControllerSlot<T>,
    ) -> bool {
        let lifecycle = self.lifecycle;
        let slot = slot(self);
        complete_if_current(lifecycle, fence, slot, controller)
    }

    fn replace<T>(
        &mut self,
        fence: ControllerFence,
        controller: T,
        slot: impl FnOnce(&mut Self) -> &mut ControllerSlot<T>,
    ) -> bool {
        if !self.is_current(fence) {
            return false;
        }
        let ControllerSlot::Ready(current) = slot(self) else {
            return false;
        };
        *current = controller;
        true
    }

    const fn slot_state(&self, kind: ControllerKind) -> ControllerSlotState {
        match kind {
            ControllerKind::ReflectedSelection => self.reflected_selection.state(),
            ControllerKind::GameData => self.game_data.state(),
            ControllerKind::AssetBrowser => self.asset_browser.state(),
            ControllerKind::Graph => self.graph.state(),
            ControllerKind::MannequinAnimation => self.mannequin_animation.state(),
            ControllerKind::Sequencer => self.sequencer.state(),
            ControllerKind::Recovery => self.recovery.state(),
            ControllerKind::SessionStatus => self.session_status.state(),
            ControllerKind::ProjectBuild => self.project_build.state(),
            ControllerKind::Runtime => self.runtime.state(),
        }
    }

    const fn failure(&self, kind: ControllerKind) -> Option<&EditorControllerFailure> {
        match kind {
            ControllerKind::ReflectedSelection => self.reflected_selection.failure(),
            ControllerKind::GameData => self.game_data.failure(),
            ControllerKind::AssetBrowser => self.asset_browser.failure(),
            ControllerKind::Graph => self.graph.failure(),
            ControllerKind::MannequinAnimation => self.mannequin_animation.failure(),
            ControllerKind::Sequencer => self.sequencer.failure(),
            ControllerKind::Recovery => self.recovery.failure(),
            ControllerKind::SessionStatus => self.session_status.failure(),
            ControllerKind::ProjectBuild => self.project_build.failure(),
            ControllerKind::Runtime => self.runtime.failure(),
        }
    }

    fn fail(&mut self, fence: ControllerFence, message: impl Into<String>) -> bool {
        let lifecycle = self.lifecycle;
        let message = message.into();
        match fence.kind {
            ControllerKind::ReflectedSelection => {
                fail_if_current(lifecycle, fence, &mut self.reflected_selection, message)
            }
            ControllerKind::GameData => {
                fail_if_current(lifecycle, fence, &mut self.game_data, message)
            }
            ControllerKind::AssetBrowser => {
                fail_if_current(lifecycle, fence, &mut self.asset_browser, message)
            }
            ControllerKind::Graph => fail_if_current(lifecycle, fence, &mut self.graph, message),
            ControllerKind::MannequinAnimation => {
                fail_if_current(lifecycle, fence, &mut self.mannequin_animation, message)
            }
            ControllerKind::Sequencer => {
                fail_if_current(lifecycle, fence, &mut self.sequencer, message)
            }
            ControllerKind::Recovery => {
                fail_if_current(lifecycle, fence, &mut self.recovery, message)
            }
            ControllerKind::SessionStatus => {
                fail_if_current(lifecycle, fence, &mut self.session_status, message)
            }
            ControllerKind::ProjectBuild => {
                fail_if_current(lifecycle, fence, &mut self.project_build, message)
            }
            ControllerKind::Runtime => {
                fail_if_current(lifecycle, fence, &mut self.runtime, message)
            }
        }
    }

    fn begin_retry(&mut self, kind: ControllerKind) -> ControllerFence {
        debug_assert_eq!(self.slot_state(kind), ControllerSlotState::Failed);
        let fence = self.lifecycle.retry(kind);
        match kind {
            ControllerKind::ReflectedSelection => {
                self.reflected_selection = ControllerSlot::Installing;
            }
            ControllerKind::GameData => self.game_data = ControllerSlot::Installing,
            ControllerKind::AssetBrowser => {
                self.asset_processor_stream.retire();
                self.asset_browser_snapshot_refresh =
                    EditorAssetBrowserSnapshotRefreshState::default();
                self.asset_browser = ControllerSlot::Installing;
            }
            ControllerKind::Graph => {
                self.graph_actions.retire();
                self.graph = ControllerSlot::Installing;
            }
            ControllerKind::MannequinAnimation => {
                self.mannequin_animation = ControllerSlot::Installing;
            }
            ControllerKind::Sequencer => self.sequencer = ControllerSlot::Installing,
            ControllerKind::Recovery => self.recovery = ControllerSlot::Installing,
            ControllerKind::SessionStatus => self.session_status = ControllerSlot::Installing,
            ControllerKind::ProjectBuild => self.project_build = ControllerSlot::Installing,
            ControllerKind::Runtime => self.runtime = ControllerSlot::Installing,
        }
        fence
    }

    fn mark_runtime_unavailable(&mut self, fence: ControllerFence) -> bool {
        if !self.is_current(fence) || fence.kind != ControllerKind::Runtime {
            return false;
        }
        self.runtime = ControllerSlot::Unavailable;
        true
    }

    fn ready<T: Clone>(
        &self,
        kind: ControllerKind,
        slot: impl FnOnce(&Self) -> &ControllerSlot<T>,
    ) -> Option<AttachedController<T>> {
        let ControllerSlot::Ready(controller) = slot(self) else {
            return None;
        };
        Some(AttachedController {
            fence: self.current_fence(kind),
            controller: controller.clone(),
        })
    }

    fn reflected_selection(
        &self,
    ) -> Option<AttachedController<EditorReflectedSelectionController>> {
        self.ready(ControllerKind::ReflectedSelection, |set| {
            &set.reflected_selection
        })
    }

    fn game_data(&self) -> Option<AttachedController<EditorGameDataCatalogController>> {
        self.ready(ControllerKind::GameData, |set| &set.game_data)
    }

    fn asset_browser(&self) -> Option<AttachedController<EditorAssetBrowserController>> {
        self.ready(ControllerKind::AssetBrowser, |set| &set.asset_browser)
    }

    fn graph(&self) -> Option<AttachedController<EditorGraphController>> {
        self.ready(ControllerKind::Graph, |set| &set.graph)
    }

    fn recovery(&self) -> Option<AttachedController<EditorRecoveryController>> {
        self.ready(ControllerKind::Recovery, |set| &set.recovery)
    }

    fn session_status(&self) -> Option<AttachedController<EditorSessionStatusController>> {
        self.ready(ControllerKind::SessionStatus, |set| &set.session_status)
    }

    fn project_build(&self) -> Option<AttachedController<EditorProjectBuildController>> {
        self.ready(ControllerKind::ProjectBuild, |set| &set.project_build)
    }

    fn runtime(&self) -> Option<AttachedController<EditorRuntimeController>> {
        self.ready(ControllerKind::Runtime, |set| &set.runtime)
    }
}

fn next_lifecycle(cx: &App) -> ControllerLifecycle {
    cx.try_global::<EditorControllers>()
        .map_or_else(ControllerLifecycle::first, |controllers| {
            ControllerLifecycle::after(controllers.lifecycle)
        })
}

/// Validate first, then publish one fresh complete set and start every
/// independent domain installer. A descriptor error leaves the prior global
/// untouched, so no partial controller generation is observable.
pub fn install_attached_controllers(
    cx: &mut App,
    session: &EditorAttachSession,
) -> Result<(), ControllerInstallPlanError> {
    validate_controller_install_plan(&CONTROLLER_INSTALL_PLAN)?;
    let lifecycle = next_lifecycle(cx);
    cx.set_global(EditorControllers::installing(session.clone(), lifecycle));
    for descriptor in CONTROLLER_INSTALL_PLAN {
        (descriptor.install)(cx, session.clone(), lifecycle.fence(descriptor.kind));
    }
    cx.refresh_windows();
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControllerRetryError {
    #[error("no attached editor controller set is installed")]
    NotAttached,
    #[error("controller `{kind}` is not failed and cannot be retried")]
    NotFailed { kind: &'static str },
}

/// Retry exactly one failed slot without replacing unrelated ready controllers.
pub fn retry_controller(cx: &mut App, kind: ControllerKind) -> Result<(), ControllerRetryError> {
    let (session, fence, runtime_available) = {
        let Some(controllers) = cx.try_global::<EditorControllers>() else {
            return Err(ControllerRetryError::NotAttached);
        };
        if controllers.slot_state(kind) != ControllerSlotState::Failed {
            return Err(ControllerRetryError::NotFailed { kind: kind.name() });
        }
        let runtime_available = controllers.session.services.runtime_host.is_some();
        let session = controllers.session.clone();
        let fence = cx.global_mut::<EditorControllers>().begin_retry(kind);
        (session, fence, runtime_available)
    };
    if kind == ControllerKind::Runtime && !runtime_available {
        cx.global_mut::<EditorControllers>()
            .mark_runtime_unavailable(fence);
        cx.refresh_windows();
        return Ok(());
    }
    let descriptor = CONTROLLER_INSTALL_PLAN
        .iter()
        .find(|descriptor| descriptor.kind == kind)
        .expect("validated controller inventory must contain every closed kind");
    (descriptor.install)(cx, session, fence);
    cx.refresh_windows();
    Ok(())
}

/// Installs the one central retry route used by controller-failure UI.
pub fn install_controller_retry_action_handler(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::RetryAttachedController, cx| {
            let kind = ControllerKind::from(action.controller);
            if let Err(error) = retry_controller(cx, kind) {
                error!(controller = kind.name(), %error, "failed to retry attached editor controller");
            }
        },
    );
}

pub fn is_current_fence(cx: &App, fence: ControllerFence) -> bool {
    cx.try_global::<EditorControllers>()
        .is_some_and(|controllers| controllers.is_current(fence))
}

/// Lists retryable failures directly from the current attached-controller
/// aggregate for the central Aether status region.
pub fn controller_failure_presentations(cx: &App) -> Vec<ControllerFailurePresentation> {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return Vec::new();
    };
    ControllerKind::ALL
        .into_iter()
        .filter_map(|kind| {
            controllers
                .failure(kind)
                .map(|failure| ControllerFailurePresentation {
                    kind,
                    message: failure.message().to_string(),
                    retry: az_editor_ui::actions::RetryAttachedController {
                        controller: kind.action_kind(),
                    },
                })
        })
        .collect()
}

pub fn fail_controller(cx: &mut App, fence: ControllerFence, message: impl Into<String>) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) {
        return false;
    }
    let failed = cx.global_mut::<EditorControllers>().fail(fence, message);
    if failed {
        cx.refresh_windows();
    }
    failed
}

pub fn mark_runtime_unavailable(cx: &mut App, fence: ControllerFence) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) {
        return false;
    }
    let unavailable = cx
        .global_mut::<EditorControllers>()
        .mark_runtime_unavailable(fence);
    if unavailable {
        cx.refresh_windows();
    }
    unavailable
}

/// The typed error a caller gets when the slot it needed is not ready.
///
/// Shared by every controller getter and by the readiness checks that want the
/// fence without a controller, so one slot state always reads the same way.
fn unready_controller_error(controllers: &EditorControllers, kind: ControllerKind) -> EditorError {
    match controllers.slot_state(kind) {
        ControllerSlotState::Installing => EditorError::ControllerInstalling {
            controller: kind.name(),
        },
        ControllerSlotState::Failed => EditorError::ControllerFailed {
            controller: kind.name(),
            message: controllers.failure(kind).map_or_else(
                || "controller installation failed".to_string(),
                |failure| failure.message().to_string(),
            ),
        },
        ControllerSlotState::Unavailable => EditorError::ControllerUnavailable {
            controller: kind.name(),
        },
        ControllerSlotState::Ready => unreachable!("ready controller must be returned"),
    }
}

macro_rules! typed_controller_api {
    (
        $getter:ident,
        $complete:ident,
        $field:ident,
        $kind:expr,
        $type:ty
    ) => {
        pub fn $getter(cx: &App) -> EditorResult<AttachedController<$type>> {
            let Some(controllers) = cx.try_global::<EditorControllers>() else {
                return Err(EditorError::MissingAttachedSession {
                    operation: "access an attached editor controller",
                });
            };
            if let Some(controller) = controllers.$field() {
                return Ok(controller);
            }
            Err(unready_controller_error(controllers, $kind))
        }

        pub fn $complete(cx: &mut App, fence: ControllerFence, controller: $type) -> bool {
            let Some(controllers) = cx.try_global::<EditorControllers>() else {
                return false;
            };
            if !controllers.is_current(fence) || fence.kind != $kind {
                return false;
            }
            cx.global_mut::<EditorControllers>()
                .complete(fence, controller, |set| &mut set.$field)
        }
    };
}

typed_controller_api!(
    reflected_selection_controller,
    complete_reflected_selection,
    reflected_selection,
    ControllerKind::ReflectedSelection,
    EditorReflectedSelectionController
);
typed_controller_api!(
    game_data_controller,
    complete_game_data,
    game_data,
    ControllerKind::GameData,
    EditorGameDataCatalogController
);
typed_controller_api!(
    asset_browser_controller,
    complete_asset_browser,
    asset_browser,
    ControllerKind::AssetBrowser,
    EditorAssetBrowserController
);
typed_controller_api!(
    graph_controller,
    complete_graph,
    graph,
    ControllerKind::Graph,
    EditorGraphController
);
pub fn complete_mannequin_animation(
    cx: &mut App,
    fence: ControllerFence,
    controller: EditorMannequinAnimationController,
) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) || fence.kind != ControllerKind::MannequinAnimation {
        return false;
    }
    cx.global_mut::<EditorControllers>()
        .complete(fence, controller, |set| &mut set.mannequin_animation)
}

pub fn complete_sequencer(
    cx: &mut App,
    fence: ControllerFence,
    controller: EditorSequencerController,
) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) || fence.kind != ControllerKind::Sequencer {
        return false;
    }
    cx.global_mut::<EditorControllers>()
        .complete(fence, controller, |set| &mut set.sequencer)
}
typed_controller_api!(
    recovery_controller,
    complete_recovery,
    recovery,
    ControllerKind::Recovery,
    EditorRecoveryController
);
typed_controller_api!(
    session_status_controller,
    complete_session_status,
    session_status,
    ControllerKind::SessionStatus,
    EditorSessionStatusController
);
typed_controller_api!(
    project_build_controller,
    complete_project_build,
    project_build,
    ControllerKind::ProjectBuild,
    EditorProjectBuildController
);
typed_controller_api!(
    runtime_controller,
    complete_runtime,
    runtime,
    ControllerKind::Runtime,
    EditorRuntimeController
);

pub fn replace_asset_browser(
    cx: &mut App,
    fence: ControllerFence,
    controller: EditorAssetBrowserController,
) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) || fence.kind != ControllerKind::AssetBrowser {
        return false;
    }
    cx.global_mut::<EditorControllers>()
        .replace(fence, controller, |set| &mut set.asset_browser)
}

pub fn replace_graph(
    cx: &mut App,
    fence: ControllerFence,
    controller: EditorGraphController,
) -> bool {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return false;
    };
    if !controllers.is_current(fence) || fence.kind != ControllerKind::Graph {
        return false;
    }
    cx.global_mut::<EditorControllers>()
        .replace(fence, controller, |set| &mut set.graph)
}

/// The fence owning a ready graph controller, without cloning the controller.
///
/// Enqueueing graph work needs proof that the slot is ready plus the fence that
/// owns it -- not the controller value. The driver takes the controller when
/// the action actually starts, which is both later and, for an action queued
/// behind others, a different controller.
pub fn ready_graph_controller_fence(cx: &App) -> EditorResult<ControllerFence> {
    let Some(controllers) = cx.try_global::<EditorControllers>() else {
        return Err(EditorError::MissingAttachedSession {
            operation: "access an attached editor controller",
        });
    };
    if controllers.slot_state(ControllerKind::Graph) == ControllerSlotState::Ready {
        return Ok(controllers.current_fence(ControllerKind::Graph));
    }
    Err(unready_controller_error(controllers, ControllerKind::Graph))
}

/// Retires the previous graph action queue and installs a fresh, empty one for
/// a graph controller that is being installed under `fence`.
pub fn install_graph_action_queue(cx: &mut App, fence: ControllerFence) {
    if !is_current_fence(cx, fence) || fence.kind != ControllerKind::Graph {
        return;
    }
    cx.global_mut::<EditorControllers>().graph_actions.install();
}

/// True while `identity` still names the live graph action queue. A completion
/// that fails this check publishes nothing and advances no queue.
pub fn graph_action_queue_is_current(
    cx: &App,
    fence: ControllerFence,
    identity: GraphActionQueueIdentity,
) -> bool {
    is_current_fence(cx, fence)
        && fence.kind == ControllerKind::Graph
        && cx
            .try_global::<EditorControllers>()
            .is_some_and(|controllers| controllers.graph_actions.is_current(identity))
}

pub fn enqueue_graph_action(
    cx: &mut App,
    fence: ControllerFence,
    action: GraphControllerAction,
) -> GraphActionAdmission {
    if !is_current_fence(cx, fence) || fence.kind != ControllerKind::Graph {
        return GraphActionAdmission::Retired;
    }
    cx.global_mut::<EditorControllers>()
        .graph_actions
        .push(action)
}

/// Takes the next pending action together with the graph controller installed
/// at this instant, so each action starts from the last successful publication
/// rather than from whatever its caller happened to hold.
pub fn start_next_graph_action(
    cx: &mut App,
    fence: ControllerFence,
    identity: GraphActionQueueIdentity,
) -> Option<(GraphControllerAction, EditorGraphController)> {
    if !graph_action_queue_is_current(cx, fence, identity) {
        return None;
    }
    let controller = cx.try_global::<EditorControllers>()?.graph()?.controller;
    let action = cx
        .global_mut::<EditorControllers>()
        .graph_actions
        .start_next(identity)?;
    Some((action, controller))
}

/// Marks the in-flight action finished. Returns whether the queue is still the
/// one that started it, which is the caller's permission to start the next.
pub fn finish_graph_action(
    cx: &mut App,
    fence: ControllerFence,
    identity: GraphActionQueueIdentity,
) -> bool {
    graph_action_queue_is_current(cx, fence, identity)
        && cx
            .global_mut::<EditorControllers>()
            .graph_actions
            .finish(identity)
}

pub fn begin_asset_processor_stream(
    cx: &mut App,
    fence: ControllerFence,
    browser_owner_id: String,
) -> Option<AssetProcessorEventStreamToken> {
    if !is_current_fence(cx, fence) || fence.kind != ControllerKind::AssetBrowser {
        return None;
    }
    Some(
        cx.global_mut::<EditorControllers>()
            .asset_processor_stream
            .begin_install(browser_owner_id),
    )
}

pub fn asset_processor_stream_is_current(
    cx: &App,
    fence: ControllerFence,
    token: AssetProcessorEventStreamToken,
) -> bool {
    is_current_fence(cx, fence)
        && cx
            .try_global::<EditorControllers>()
            .is_some_and(|controllers| controllers.asset_processor_stream.is_current(token))
}

pub fn retain_asset_processor_stream(
    cx: &mut App,
    fence: ControllerFence,
    token: AssetProcessorEventStreamToken,
    subscription: AssetProcessorEventSubscription,
) -> Result<(), AssetProcessorEventSubscription> {
    if !is_current_fence(cx, fence) {
        return Err(subscription);
    }
    cx.global_mut::<EditorControllers>()
        .asset_processor_stream
        .retain_subscription(token, subscription)
}

pub fn admit_asset_processor_stream_initial(
    cx: &mut App,
    fence: ControllerFence,
    token: AssetProcessorEventStreamToken,
    event_watermark: u64,
) -> bool {
    is_current_fence(cx, fence)
        && cx
            .global_mut::<EditorControllers>()
            .asset_processor_stream
            .admit_initial(token, event_watermark)
}

pub fn admit_asset_processor_stream_event(
    cx: &mut App,
    fence: ControllerFence,
    token: AssetProcessorEventStreamToken,
    event_seq: u64,
) -> AssetProcessorEventAdmission {
    if !is_current_fence(cx, fence) {
        return AssetProcessorEventAdmission::Superseded;
    }
    cx.global_mut::<EditorControllers>()
        .asset_processor_stream
        .admit_event(token, event_seq)
}

pub fn finish_asset_processor_stream(
    cx: &mut App,
    fence: ControllerFence,
    token: AssetProcessorEventStreamToken,
) -> bool {
    is_current_fence(cx, fence)
        && cx
            .global_mut::<EditorControllers>()
            .asset_processor_stream
            .finish(token)
}

pub fn asset_processor_event_stream_cursor(
    cx: &App,
    browser_owner_id: &str,
) -> Option<AssetProcessorEventStreamCursor> {
    cx.try_global::<EditorControllers>()?
        .asset_processor_stream
        .cursor(browser_owner_id)
}

pub fn asset_processor_snapshot_admission(
    cx: &App,
    browser_owner_id: &str,
    cursor: Option<AssetProcessorEventStreamCursor>,
) -> AssetProcessorSnapshotAdmission {
    cx.try_global::<EditorControllers>().map_or(
        AssetProcessorSnapshotAdmission::Superseded,
        |controllers| {
            controllers
                .asset_processor_stream
                .snapshot_admission(browser_owner_id, cursor)
        },
    )
}

pub fn request_asset_browser_snapshot_refresh(
    cx: &mut App,
    fence: ControllerFence,
    session: &EditorAttachSession,
    reason: &'static str,
) -> Option<AssetBrowserSnapshotRefreshRequest> {
    if !is_current_fence(cx, fence) {
        return None;
    }
    Some(
        cx.global_mut::<EditorControllers>()
            .asset_browser_snapshot_refresh
            .request(session, fence, reason),
    )
}

pub fn complete_asset_browser_snapshot_refresh(
    cx: &mut App,
    fence: ControllerFence,
    session_id: &str,
) -> Option<PendingAssetBrowserSnapshotRefresh> {
    if !is_current_fence(cx, fence) {
        return None;
    }
    cx.global_mut::<EditorControllers>()
        .asset_browser_snapshot_refresh
        .complete(session_id)
}

// Everything below is test-only.
//
// Two guards read this file, and both want the same shape. The retired-path
// guard treats the text before the first `#[cfg(test)]` as production, so test
// seams belong at the end. The universal-IPC-client guard only strips
// `#[cfg(test)] mod` blocks, so a bare `#[cfg(test)] fn` still reads as
// production -- test seams must live inside the module below, not beside it.

#[cfg(test)]
pub use self::test_support::{
    install_ready_graph_slot_for_tests, installed_graph_controller, pending_graph_action_count,
    retire_graph_action_queue_for_tests,
};

#[cfg(test)]
mod test_support {
    use super::*;

    /// A structurally complete attach session with no live services behind it.
    ///
    /// The attached-controller aggregate needs a session value to exist; graph
    /// queue tests need the aggregate, not the services. Every field is a
    /// plausible but inert value so nothing in a test can accidentally reach a
    /// real endpoint.
    fn attach_session_for_tests(run_dir: &std::path::Path) -> EditorAttachSession {
        use az_proto_core::{Endpoint, EndpointKind, ProtocolVersion, ServiceId, ServiceRole};

        fn descriptor(name: &str, role: ServiceRole) -> az_proto_core::ServiceDescriptor {
            az_proto_core::ServiceDescriptor {
                id: ServiceId::new("az.editor.tests", name),
                role,
                run: uuid::Uuid::nil(),
                protocol: ProtocolVersion::CURRENT,
                endpoint: Endpoint::new(EndpointKind::InProcess, format!("az.editor.tests/{name}")),
                observability_endpoint: None,
                lifecycle_endpoint: None,
                capabilities: Vec::new(),
            }
        }

        EditorAttachSession {
            project_id: "az.editor.tests".to_owned(),
            project_root: run_dir.to_path_buf(),
            daemon_endpoint: Endpoint::new(EndpointKind::InProcess, "az.editor.tests/daemon"),
            session_id: uuid::Uuid::nil(),
            session_slug: "editor-tests".to_owned(),
            workspace: crate::attach::AttachedWorkspace {
                project_id: "az.editor.tests".to_owned(),
                workspace_root: run_dir.display().to_string(),
                branch: "main".to_owned(),
            },
            source_status: az_source_control::SourceStatus {
                repository_id: "az.editor.tests".to_owned(),
                branch: Some("main".to_owned()),
                revision_number: None,
                revision_id: None,
                remote_revision_number: None,
                remote_revision_id: None,
                in_sync_with_remote: true,
                changed_lines: Vec::new(),
                raw_output: String::new(),
            },
            run_dir: run_dir.to_path_buf(),
            session_supervisor: descriptor("session-supervisor", ServiceRole::SessionSupervisor),
            services: crate::attach::EditorAttachServices {
                project_host: descriptor("project-host", ServiceRole::ProjectHost),
                asset_processor: descriptor("asset-processor", ServiceRole::AssetProcessor),
                runtime_host: None,
            },
            workspace_snapshot: az_proto_asset::WorkspaceSnapshot {
                workspace_id: 1,
                project_id: "az.editor.tests".to_owned(),
                workspace_root: run_dir.display().to_string(),
                branch: "main".to_owned(),
                created_unix_ms: 0,
                updated_unix_ms: 0,
                roots: Vec::new(),
            },
            type_registry: az_proto_project::vnext::TypeRegistrySnapshot {
                schema_catalog_hash: Vec::new(),
                types: Vec::new(),
            },
            gamedata_catalog: az_proto_project::GameDataCatalogSnapshot::empty(0),
            project_inventory: az_proto_project::ProjectInventoryReport {
                service_role: "project-host".to_owned(),
                lock_status: az_proto_project::ProjectInventoryLockStatus {
                    state: az_proto_project::ProjectInventoryLockState::Fresh,
                    path: String::new(),
                    diagnostic: String::new(),
                },
                gems: Vec::new(),
                registry: az_proto_project::ProjectInventoryRegistryCounts {
                    build_rules: 0,
                    node_types: 0,
                    graph_types: 0,
                },
                diagnostics: Vec::new(),
                degraded: false,
            },
            verification: crate::attach::EditorAttachVerification {
                project_host_type_count: 0,
                asset_processor_workspace_id: 1,
                asset_processor_root_count: 0,
                runtime_host_verified: false,
                runtime_host_projection_count: None,
                project_inventory_expected_gem_count: 0,
                project_inventory_active_gem_count: 0,
                project_inventory_diagnostics_count: 0,
                project_inventory_degraded: false,
            },
        }
    }

    /// Publishes a fresh aggregate whose graph slot is already `Ready`, mirroring
    /// what a completed graph install leaves behind: a new lifecycle generation and
    /// a fresh, empty action queue. Calling it twice models reattachment.
    pub fn install_ready_graph_slot_for_tests(
        cx: &mut App,
        controller: EditorGraphController,
    ) -> ControllerFence {
        let lifecycle = next_lifecycle(cx);
        let session = attach_session_for_tests(&std::env::temp_dir());
        let mut controllers = EditorControllers::installing(session, lifecycle);
        controllers.graph = ControllerSlot::Ready(controller);
        controllers.graph_actions.install();
        cx.set_global(controllers);
        lifecycle.fence(ControllerKind::Graph)
    }

    /// Retires the live graph action queue the way a same-session retry does,
    /// without needing a failed slot to retry from.
    pub fn retire_graph_action_queue_for_tests(cx: &mut App) {
        cx.global_mut::<EditorControllers>().graph_actions.retire();
    }

    /// The number of accepted-but-unstarted graph actions.
    pub fn pending_graph_action_count(cx: &App) -> usize {
        cx.try_global::<EditorControllers>()
            .map_or(0, |controllers| controllers.graph_actions.pending_len())
    }

    /// The graph controller currently installed in the aggregate, if any.
    pub fn installed_graph_controller(cx: &App) -> Option<EditorGraphController> {
        Some(cx.try_global::<EditorControllers>()?.graph()?.controller)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_op(_: &mut App, _: EditorAttachSession, _: ControllerFence) {}

    #[test]
    fn production_plan_has_the_closed_exact_inventory() {
        validate_controller_install_plan(&CONTROLLER_INSTALL_PLAN).unwrap();
        assert_eq!(CONTROLLER_INSTALL_PLAN.len(), ControllerKind::ALL.len());
    }

    #[test]
    fn plan_validation_rejects_missing_duplicate_and_policy_mismatch_before_publication() {
        let missing = &CONTROLLER_INSTALL_PLAN[..CONTROLLER_INSTALL_PLAN.len() - 1];
        assert_eq!(
            validate_controller_install_plan(missing),
            Err(ControllerInstallPlanError::Missing { kind: "runtime" })
        );

        let mut duplicate = CONTROLLER_INSTALL_PLAN;
        duplicate[ControllerKind::Runtime.index()] = ControllerDescriptor {
            kind: ControllerKind::Graph,
            policy: ControllerPolicy::Required,
            install: no_op,
        };
        assert_eq!(
            validate_controller_install_plan(&duplicate),
            Err(ControllerInstallPlanError::Duplicate { kind: "graph" })
        );

        let mut policy_mismatch = CONTROLLER_INSTALL_PLAN;
        policy_mismatch[ControllerKind::Runtime.index()] = ControllerDescriptor {
            kind: ControllerKind::Runtime,
            policy: ControllerPolicy::Required,
            install: no_op,
        };
        assert_eq!(
            validate_controller_install_plan(&policy_mismatch),
            Err(ControllerInstallPlanError::PolicyMismatch { kind: "runtime" })
        );
    }

    #[test]
    fn retry_fences_an_older_same_generation_completion() {
        let mut lifecycle = ControllerLifecycle::first();
        let original = lifecycle.fence(ControllerKind::Graph);
        let retry = lifecycle.retry(ControllerKind::Graph);

        assert_eq!(original.generation, retry.generation);
        assert_ne!(original.attempt, retry.attempt);
        assert!(!lifecycle.is_current(original));
        assert!(lifecycle.is_current(retry));
    }

    #[test]
    fn retry_changes_only_the_selected_slot_attempt() {
        let mut lifecycle = ControllerLifecycle::first();
        let original = ControllerKind::ALL.map(|kind| (kind, lifecycle.fence(kind)));
        let retry = lifecycle.retry(ControllerKind::AssetBrowser);

        for (kind, fence) in original {
            if kind == ControllerKind::AssetBrowser {
                assert!(!lifecycle.is_current(fence));
                assert!(lifecycle.is_current(retry));
            } else {
                assert!(
                    lifecycle.is_current(fence),
                    "retrying Asset Browser must not invalidate {kind:?}"
                );
            }
        }
    }

    #[test]
    fn injected_old_completion_cannot_replace_a_retried_slot() {
        let mut lifecycle = ControllerLifecycle::first();
        let original = lifecycle.fence(ControllerKind::AssetBrowser);
        let retry = lifecycle.retry(ControllerKind::AssetBrowser);
        let mut slot = ControllerSlot::Installing;

        assert!(
            !complete_if_current(lifecycle, original, &mut slot, "stale controller"),
            "a completion injected from the previous attempt must be fenced"
        );
        assert!(matches!(slot, ControllerSlot::Installing));
        assert!(complete_if_current(
            lifecycle,
            retry,
            &mut slot,
            "fresh controller"
        ));
        assert!(matches!(slot, ControllerSlot::Ready("fresh controller")));
    }

    #[test]
    fn one_runtime_installer_failure_leaves_an_unrelated_ready_slot_usable() {
        let lifecycle = ControllerLifecycle::first();
        let mut runtime = ControllerSlot::<()>::Installing;
        let mut graph = ControllerSlot::<&str>::Installing;

        assert!(complete_if_current(
            lifecycle,
            lifecycle.fence(ControllerKind::Graph),
            &mut graph,
            "ready graph",
        ));
        assert!(fail_if_current(
            lifecycle,
            lifecycle.fence(ControllerKind::Runtime),
            &mut runtime,
            "runtime RPC refused the verified endpoint",
        ));

        assert_eq!(runtime.state(), ControllerSlotState::Failed);
        assert_eq!(graph.state(), ControllerSlotState::Ready);
        assert!(matches!(graph, ControllerSlot::Ready("ready graph")));
    }

    #[test]
    fn absent_runtime_endpoint_is_unavailable_not_a_failed_controller() {
        let runtime = initial_runtime_slot::<()>(false);

        assert_eq!(runtime.state(), ControllerSlotState::Unavailable);
        assert!(runtime.failure().is_none());
    }

    #[test]
    fn reattach_fences_every_old_slot_completion() {
        let first = ControllerLifecycle::first();
        let second = ControllerLifecycle::after(first);

        for kind in ControllerKind::ALL {
            assert!(!second.is_current(first.fence(kind)));
            assert!(second.is_current(second.fence(kind)));
        }
    }

    #[test]
    fn typed_retry_action_targets_the_closed_controller_inventory() {
        use az_editor_ui::actions::AttachedControllerKind as ActionKind;

        let targets = [
            (
                ActionKind::ReflectedSelection,
                ControllerKind::ReflectedSelection,
            ),
            (ActionKind::GameData, ControllerKind::GameData),
            (ActionKind::AssetBrowser, ControllerKind::AssetBrowser),
            (ActionKind::Graph, ControllerKind::Graph),
            (
                ActionKind::MannequinAnimation,
                ControllerKind::MannequinAnimation,
            ),
            (ActionKind::Sequencer, ControllerKind::Sequencer),
            (ActionKind::Recovery, ControllerKind::Recovery),
            (ActionKind::SessionStatus, ControllerKind::SessionStatus),
            (ActionKind::ProjectBuild, ControllerKind::ProjectBuild),
            (ActionKind::Runtime, ControllerKind::Runtime),
        ];

        for (action, kind) in targets {
            assert_eq!(ControllerKind::from(action), kind);
        }
    }

    /// Per-controller globals and install paths the typed `EditorControllers`
    /// aggregate replaced. Second column is the reintroduction each needle must
    /// still fire on; the control below iterates this same table.
    const RETIRED_CONTROLLER_PATHS: &[(&str, &str)] = &[
        (
            "impl Global for EditorAssetBrowserController",
            "impl Global for EditorAssetBrowserController {}",
        ),
        (
            "impl Global for EditorReflectedSelectionController",
            "impl Global for EditorReflectedSelectionController {}",
        ),
        (
            "impl Global for EditorGameDataCatalogController",
            "impl Global for EditorGameDataCatalogController {}",
        ),
        (
            "impl Global for EditorGraphController",
            "impl Global for EditorGraphController {}",
        ),
        (
            "impl Global for EditorRecoveryController",
            "impl Global for EditorRecoveryController {}",
        ),
        (
            "impl Global for EditorSessionStatusController",
            "impl Global for EditorSessionStatusController {}",
        ),
        (
            "impl Global for EditorProjectBuildController",
            "impl Global for EditorProjectBuildController {}",
        ),
        (
            "impl Global for EditorRuntimeController",
            "impl Global for EditorRuntimeController {}",
        ),
        (
            "impl Global for EditorAssetProcessorEventStreamOwner",
            "impl Global for EditorAssetProcessorEventStreamOwner {}",
        ),
        (
            "impl Global for EditorAssetBrowserSnapshotRefreshState",
            "impl Global for EditorAssetBrowserSnapshotRefreshState {}",
        ),
        (
            "SequenceControllerGeneration",
            "cx.set_global(SequenceControllerGeneration(next));",
        ),
        (
            "install_asset_browser_controller(",
            "install_asset_browser_controller(cx, session.clone());",
        ),
        (
            "install_reflected_selection_controller(",
            "install_reflected_selection_controller(cx, session.clone());",
        ),
        (
            "install_game_data_catalog_controller(",
            "install_game_data_catalog_controller(cx, session.clone());",
        ),
        (
            "install_graph_controller(",
            "install_graph_controller(cx, session.clone());",
        ),
        (
            "install_mannequin_animation_controller(",
            "install_mannequin_animation_controller(cx, session.clone());",
        ),
        (
            "install_sequencer_controller(",
            "install_sequencer_controller(cx, session.clone());",
        ),
        (
            "install_recovery_controller(",
            "install_recovery_controller(cx, session.clone());",
        ),
        (
            "install_session_status_controller(",
            "install_session_status_controller(cx, session.clone());",
        ),
        (
            "install_project_build_controller(",
            "install_project_build_controller(cx, session.clone());",
        ),
        (
            "install_runtime_controller(",
            "install_runtime_controller(cx, session.clone());",
        ),
        // Graph actions run through the session queue now. This helper was
        // the last-finished-wins spawn path the queue replaced.
        (
            "spawn_graph_controller_action(",
            "fn spawn_graph_controller_action(cx: &mut App) {}",
        ),
    ];

    #[test]
    fn retired_controller_globals_and_install_paths_cannot_return() {
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let sources = [
            "asset_processor/install.rs",
            "authored_selection.rs",
            "game_data_catalog.rs",
            "graph_ui.rs",
            "mannequin_animation.rs",
            "sequencer.rs",
            "recovery.rs",
            "session_supervisor.rs",
            "project_build.rs",
            "runtime_host.rs",
            "app.rs",
            "lib.rs",
            "project_workflow.rs",
            "viewport_host.rs",
        ]
        .into_iter()
        .map(|path| {
            std::fs::read_to_string(source_root.join(path))
                .unwrap_or_else(|error| panic!("read {path}: {error}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
        let controller_set = std::fs::read_to_string(source_root.join("controller_set.rs"))
            .expect("read controller_set.rs");
        let controller_set_production = controller_set
            .split("#[cfg(test)]")
            .next()
            .expect("controller_set source always has a production prefix");
        let sources = format!("{sources}\n{controller_set_production}");

        let aggregate_default_global = format!("{}{}", "default_global::<", "EditorControllers>");
        for retired in RETIRED_CONTROLLER_PATHS
            .iter()
            .map(|&(retired, _)| retired)
            .chain(std::iter::once(aggregate_default_global.as_str()))
        {
            assert!(
                !az_architecture_guard::symbols_contain(&sources, retired),
                "retired attached-controller path returned: {retired}"
            );
        }
    }

    /// Positive control for the guard above: every needle fires on the retired
    /// global or install path it forbids, and none of them matches the
    /// aggregate slot shapes that replaced them. The control iterates the same
    /// table the guard does, so a needle cannot be added without a
    /// reintroduction that proves it can fail (ticket 042); previously one
    /// needle out of twenty-three carried the whole list's credibility.
    #[test]
    fn retired_controller_needles_fire_and_spare_the_aggregate_slots() {
        const AGGREGATE_SLOT_SHAPES: &[&str] = &[
            "install: crate::game_data_catalog::install_game_data_catalog_slot,",
            "let controllers = cx.global::<EditorControllers>();",
            "impl Global for EditorControllers {}",
            "controllers.asset_browser().map(|controller| controller.client())",
        ];

        for &(retired, reintroduction) in RETIRED_CONTROLLER_PATHS {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, retired),
                "needle `{retired}` cannot match the retired path it forbids: {reintroduction}"
            );
            for shape in AGGREGATE_SLOT_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, retired),
                    "needle `{retired}` rejects the aggregate slot shape `{shape}`"
                );
            }
        }

        let aggregate_default_global = format!("{}{}", "default_global::<", "EditorControllers>");
        assert!(
            az_architecture_guard::symbols_contain(
                "let controllers = cx.default_global::<EditorControllers>();",
                &aggregate_default_global,
            ),
            "the aggregate needle cannot match the default-global read it forbids"
        );
        assert!(
            !az_architecture_guard::symbols_contain(
                "let controllers = cx.global::<EditorControllers>();",
                &aggregate_default_global,
            ),
            "the aggregate needle rejects the installed-aggregate read that replaced it"
        );
    }

    #[test]
    fn central_status_surface_dispatches_the_typed_retry_action() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("app")
                .join("aether_editor_view.rs"),
        )
        .expect("read aether editor view");

        assert!(
            az_architecture_guard::symbols_contain(&source, "controller_failure_presentations(cx)",),
            "central status must derive failures from EditorControllers"
        );
        assert!(
            az_architecture_guard::symbols_contain(
                &source,
                "window.dispatch_action(Box::new(retry.clone()), cx)",
            ),
            "central status must dispatch the typed retry action"
        );
    }
}
