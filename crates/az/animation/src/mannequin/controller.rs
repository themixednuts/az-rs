//! Renderer-independent action-controller and scope arbitration.

use super::{
    Action, ActionArena, ActionEndMethod, ActionFailure, ActionFlags, ActionHandle, ActionQueue,
    ActionStatus, ClipType, ControllerFlags, FragmentTagState, InvalidActionHandle,
    MannequinAction, MannequinParameter, MannequinParameterSource, MannequinParameters,
    ProceduralDebug, ResumeFlags, ScopeContextId, ScopeEvent, ScopeId, ScopeMask, TagState,
};
use arrayvec::ArrayVec;
use az_core::crc::Crc32;
use smallvec::SmallVec;

/// Shipping controller limit: scope masks are 32-bit values.
pub const MAX_SCOPES: usize = u32::BITS as usize;
const NORMAL_RESOLVE_ITERATIONS: usize = 5;
const PRE_UPDATE_RESOLVE_ITERATIONS: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Mannequin controller has {actual} scopes; the shipping maximum is {MAX_SCOPES}")]
pub struct ScopeCountError {
    pub actual: usize,
}

/// Runtime state owned by one Cry Mannequin scope.
#[derive(Debug, Clone)]
pub struct ActionScope {
    pub id: ScopeId,
    pub context: ScopeContextId,
    pub base_layer: u32,
    pub layer_count: u32,
    pub action: Option<ActionHandle>,
    pub exiting_action: Option<ActionHandle>,
    pub one_shot: bool,
    pub fragment_time: f32,
    pub blend_out_duration: f32,
}

impl ActionScope {
    #[must_use]
    pub const fn new(
        id: ScopeId,
        context: ScopeContextId,
        base_layer: u32,
        layer_count: u32,
    ) -> Self {
        Self {
            id,
            context,
            base_layer,
            layer_count,
            action: None,
            exiting_action: None,
            one_shot: false,
            fragment_time: 0.0,
            blend_out_duration: 0.0,
        }
    }

    #[must_use]
    pub fn playing_action(&self) -> Option<ActionHandle> {
        self.exiting_action.or(self.action)
    }
}

// `ScopeEvent` carries `ActionMutation::SetSpeedBias(f32)`, so it can never be
// `Eq`; this outcome therefore derives only `PartialEq`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallOutcome {
    pub installed_scopes: ScopeMask,
    /// Scopes occupied by an overlapping action but not claimed by the new
    /// action. Cry queues an empty fragment on these before detaching them.
    pub cleared_scopes: ScopeMask,
    pub root_scope: ScopeId,
    pub root_has_outro_transition: bool,
    pub ending_actions: SmallVec<[EndingAction; MAX_SCOPES]>,
    /// Events emitted by procedural clips entered synchronously during install.
    pub scope_events: SmallVec<[ScopeEvent; MAX_SCOPES]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndingAction {
    pub handle: ActionHandle,
    pub root_scope: ScopeId,
    pub transition_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeInstallComparison {
    pub scope: ScopeId,
    pub current: Option<ActionHandle>,
    pub current_status: Option<ActionStatus>,
    pub priority: super::PriorityComparison,
    pub is_requeue: bool,
    /// False for a non-root scope owned by another multi-scope action. Cry
    /// times that action only on its root scope.
    pub times_transition: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallReadiness {
    Waiting { time_remaining: f32 },
    Ready { time_remaining: f32 },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ActionDuration {
    pub fragment: f32,
    pub transition: f32,
}

impl InstallReadiness {
    #[must_use]
    pub const fn time_remaining(self) -> f32 {
        match self {
            Self::Waiting { time_remaining } | Self::Ready { time_remaining } => time_remaining,
        }
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Domain bridge used by the engine state machine.
///
/// Fragment selection and clip installation are asset/runtime-backend concerns;
/// priority, queue lifetime, scope conflicts, action flags, and update order stay
/// in the controller below.
pub trait ActionControllerDriver<A>
where
    A: MannequinAction,
{
    fn query_scope_mask(
        &mut self,
        action: &A,
        tags: FragmentTagState,
        active_scopes: ScopeMask,
    ) -> ScopeMask;

    fn on_resolve_action_installations(&mut self, _handle: ActionHandle, _action: &mut A) -> bool {
        false
    }

    fn request_install(
        &mut self,
        _handle: ActionHandle,
        _action: &mut A,
        _requested_scopes: ScopeMask,
        _scopes: &mut [ActionScope],
    ) {
    }

    fn snapshot_scope_state(&mut self, _scopes: &mut [ActionScope]) {}

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the state CActionController weighs when it decides whether an action may install"
    )]
    fn can_install(
        &mut self,
        handle: ActionHandle,
        action: &A,
        requested_scopes: ScopeMask,
        affected_scopes: ScopeMask,
        scopes: &[ActionScope],
        comparisons: &[ScopeInstallComparison],
        delta_time: f32,
    ) -> Option<InstallReadiness>;

    fn install(
        &mut self,
        handle: ActionHandle,
        action: &mut A,
        requested_scopes: ScopeMask,
        scopes: &mut [ActionScope],
        time_remaining: f32,
    ) -> InstallOutcome;

    fn blend_off_actions(
        &mut self,
        _actions: &mut ActionArena<A>,
        _scopes: &mut [ActionScope],
        _active_scopes: ScopeMask,
        _delta_time: f32,
        _ending_actions: &mut impl Extend<EndingAction>,
    ) {
    }

    fn on_action_updated(&mut self, _handle: ActionHandle, _action: &mut A, _status: ActionStatus) {
    }

    fn update_scope(
        &mut self,
        _scope: &mut ActionScope,
        _playing_action: Option<(ActionHandle, f32, f32)>,
        _delta_time: f32,
        _procedural_debug: ProceduralDebug,
        _events: &mut impl Extend<ScopeEvent>,
    ) {
    }

    fn update_procedural_contexts(&mut self, _delta_time: f32) {}

    fn pause_scope(&mut self, _scope: &ActionScope) {}

    fn resume_scope(&mut self, _scope: &ActionScope, _flags: ResumeFlags) {}

    fn flush_scope(&mut self, _scope: &ActionScope, _method: ActionEndMethod) {}

    fn scope_needs_install(&self, scope: &ActionScope, installed_contexts: u32) -> bool {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "scope context indices are bounded by MAX_SCOPES (32) and the \
                      shift is guarded by checked_shl regardless"
        )]
        let context_shift = scope.context.index() as u32;
        let context_bit = 1_u32.checked_shl(context_shift).unwrap_or(0);
        context_bit == 0 || installed_contexts & context_bit == 0
    }

    fn is_scope_different(
        &self,
        _scope: &ActionScope,
        _fragment: super::FragmentId,
        _fragment_tags: TagState,
    ) -> bool {
        true
    }

    fn query_duration(
        &mut self,
        _handle: ActionHandle,
        _action: &mut A,
        _requested_scopes: ScopeMask,
        _scopes: &[ActionScope],
        _comparisons: &[ScopeInstallComparison],
    ) -> Option<ActionDuration> {
        None
    }
}

#[derive(Debug)]
pub struct ActionController<A> {
    actions: ActionArena<A>,
    queue: ActionQueue,
    scopes: Vec<ActionScope>,
    active_scopes: ScopeMask,
    global_tags: TagState,
    flags: ControllerFlags,
    time_scale: f32,
    parameters: MannequinParameters,
    scratch: ControllerScratch,
}

#[derive(Debug, Default)]
struct ControllerScratch {
    handles: Vec<ActionHandle>,
    auxiliary_handles: Vec<ActionHandle>,
    ticked_actions: Vec<ActionHandle>,
    triggered_actions: Vec<(ActionHandle, bool, bool)>,
    ending_actions: Vec<EndingAction>,
    scope_events: Vec<ScopeEvent>,
}

impl<A> ActionController<A>
where
    A: MannequinAction,
{
    /// Builds a controller over `scopes`, which must be ordered by scope id.
    ///
    /// # Errors
    ///
    /// Returns [`ScopeCountError`] when more than [`MAX_SCOPES`] scopes are
    /// supplied, because the scope mask is a single `u32`.
    pub fn new(scopes: impl IntoIterator<Item = ActionScope>) -> Result<Self, ScopeCountError> {
        let scopes = scopes.into_iter().collect::<Vec<_>>();
        if scopes.len() > MAX_SCOPES {
            return Err(ScopeCountError {
                actual: scopes.len(),
            });
        }
        debug_assert!(
            scopes
                .iter()
                .enumerate()
                .all(|(index, scope)| scope.id.index() == index)
        );

        let active_scopes = if scopes.is_empty() {
            ScopeMask::empty()
        } else if scopes.len() == MAX_SCOPES {
            ScopeMask::ALL
        } else {
            ScopeMask::from_bits_retain((1_u32 << scopes.len()) - 1)
        };

        let scope_capacity = scopes.len();
        let action_capacity = scope_capacity + Action::MAX_QUEUE_SIZE;
        Ok(Self {
            actions: ActionArena::with_capacity(action_capacity),
            queue: ActionQueue::with_capacity(Action::MAX_QUEUE_SIZE),
            scopes,
            active_scopes,
            global_tags: TagState::EMPTY,
            flags: ControllerFlags::empty(),
            time_scale: 1.0,
            parameters: MannequinParameters::default(),
            scratch: ControllerScratch {
                handles: Vec::with_capacity(action_capacity),
                auxiliary_handles: Vec::with_capacity(action_capacity),
                ticked_actions: Vec::with_capacity(Action::MAX_QUEUE_SIZE),
                triggered_actions: Vec::with_capacity(Action::MAX_QUEUE_SIZE),
                ending_actions: Vec::with_capacity(scope_capacity),
                scope_events: Vec::with_capacity(4),
            },
        })
    }

    #[must_use]
    pub const fn actions(&self) -> &ActionArena<A> {
        &self.actions
    }

    pub const fn actions_mut(&mut self) -> &mut ActionArena<A> {
        &mut self.actions
    }

    #[must_use]
    pub fn scopes(&self) -> &[ActionScope] {
        &self.scopes
    }

    pub fn scopes_mut(&mut self) -> &mut [ActionScope] {
        &mut self.scopes
    }

    #[must_use]
    pub fn scope(&self, scope: ScopeId) -> Option<&ActionScope> {
        self.scopes.get(scope.index())
    }

    pub fn scope_mut(&mut self, scope: ScopeId) -> Option<&mut ActionScope> {
        self.scopes.get_mut(scope.index())
    }

    #[must_use]
    pub const fn active_scopes(&self) -> ScopeMask {
        self.active_scopes
    }

    pub const fn set_active_scopes(&mut self, scopes: ScopeMask) {
        self.active_scopes = scopes;
    }

    #[must_use]
    pub fn is_scope_active(&self, scope: ScopeId) -> bool {
        self.active_scopes.contains_scope(scope)
    }

    #[must_use]
    pub const fn global_tags(&self) -> TagState {
        self.global_tags
    }

    pub const fn set_global_tags(&mut self, tags: TagState) {
        self.global_tags = tags;
    }

    #[must_use]
    pub const fn flags(&self) -> ControllerFlags {
        self.flags
    }

    pub fn set_flag(&mut self, flag: ControllerFlags, enabled: bool) {
        self.flags.set(flag, enabled);
    }

    #[must_use]
    pub const fn time_scale(&self) -> f32 {
        self.time_scale
    }

    pub const fn set_time_scale(&mut self, time_scale: f32) {
        self.time_scale = time_scale;
    }

    pub fn pause(&mut self, driver: &mut impl ActionControllerDriver<A>) {
        if self.flags.contains(ControllerFlags::PAUSED_UPDATE) {
            return;
        }
        self.flags.insert(ControllerFlags::PAUSED_UPDATE);
        for scope in &self.scopes {
            driver.pause_scope(scope);
        }
    }

    pub fn resume(&mut self, flags: ResumeFlags, driver: &mut impl ActionControllerDriver<A>) {
        if !self.flags.contains(ControllerFlags::PAUSED_UPDATE) {
            return;
        }
        for scope in &self.scopes {
            driver.resume_scope(scope, flags);
        }
        self.flags.remove(ControllerFlags::PAUSED_UPDATE);
    }

    #[must_use]
    pub const fn parameters(&self) -> &MannequinParameters {
        &self.parameters
    }

    pub const fn parameters_mut(&mut self) -> &mut MannequinParameters {
        &mut self.parameters
    }

    #[must_use]
    pub fn parameter(&self, name: impl Into<Crc32>) -> Option<&bevy_math::Isometry3d> {
        self.parameters.get(name)
    }

    pub fn set_parameter(
        &mut self,
        name: impl Into<Crc32>,
        value: impl Into<bevy_math::Isometry3d>,
    ) {
        self.parameters.set(name, value);
    }

    pub fn set_mannequin_parameter(&mut self, parameter: MannequinParameter) {
        self.parameters.set_parameter(parameter);
    }

    pub fn remove_parameter(&mut self, name: impl Into<Crc32>) -> Option<MannequinParameter> {
        self.parameters.remove(name)
    }

    pub fn reset_parameters(&mut self) {
        self.parameters.clear();
    }

    pub fn insert_action(&mut self, action: A) -> ActionHandle {
        self.actions.insert(action)
    }

    pub fn remove_action(&mut self, handle: ActionHandle) -> Option<A> {
        self.queue.remove(handle);
        for scope in &mut self.scopes {
            if scope.action == Some(handle) {
                scope.action = None;
            }
            if scope.exiting_action == Some(handle) {
                scope.exiting_action = None;
            }
        }
        self.actions.remove(handle)
    }

    /// Deliver a procedural `ActionEvent` to its controlling `IAction`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action.
    pub fn send_action_event(
        &mut self,
        handle: ActionHandle,
        event: Crc32,
    ) -> Result<(), InvalidActionHandle> {
        let action = self.actions.get_mut(handle).ok_or(InvalidActionHandle)?;
        action.on_action_event(event);
        Ok(())
    }

    /// Places `handle` on the pending queue, optionally at `queue_time`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action.
    pub fn queue(
        &mut self,
        handle: ActionHandle,
        queue_time: Option<f32>,
    ) -> Result<(), InvalidActionHandle> {
        self.queue.queue(&mut self.actions, handle, queue_time)
    }

    /// Re-queues an action that is already installed or pending.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action.
    pub fn requeue(&mut self, handle: ActionHandle) -> Result<(), InvalidActionHandle> {
        self.queue.requeue(&mut self.actions, handle)
    }

    /// Asks the driver how long the action's fragment and transition would run.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action.
    pub fn query_duration(
        &mut self,
        handle: ActionHandle,
        driver: &mut impl ActionControllerDriver<A>,
    ) -> Result<Option<ActionDuration>, InvalidActionHandle> {
        let action = self.actions.get(handle).ok_or(InvalidActionHandle)?;
        let tags = FragmentTagState {
            global_tags: self.global_tags,
            fragment_tags: action.as_ref().fragment_tags,
        };
        let requested = (action.as_ref().forced_scope_mask
            | driver.query_scope_mask(action, tags, self.active_scopes))
            & self.active_scopes;
        let comparisons = self.scope_install_comparisons(handle, requested);
        let action = self.actions.get_mut(handle).ok_or(InvalidActionHandle)?;
        Ok(driver.query_duration(handle, action, requested, &self.scopes, &comparisons))
    }

    /// Asks the driver whether `handle` can take `scopes` this frame.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action.
    pub fn can_install(
        &mut self,
        handle: ActionHandle,
        scopes: ScopeMask,
        delta_time: f32,
        driver: &mut impl ActionControllerDriver<A>,
    ) -> Result<Option<InstallReadiness>, InvalidActionHandle> {
        self.actions.get(handle).ok_or(InvalidActionHandle)?;
        let affected_scopes = self.expand_overlapping_scopes(scopes) & self.active_scopes;
        if !self.can_install_by_priority(handle, affected_scopes) {
            return Ok(None);
        }
        let comparisons = self.scope_install_comparisons(handle, affected_scopes);
        let action = self.actions.get(handle).ok_or(InvalidActionHandle)?;
        Ok(driver.can_install(
            handle,
            action,
            scopes & self.active_scopes,
            affected_scopes,
            &self.scopes,
            &comparisons,
            delta_time,
        ))
    }

    /// Finish one detached action using the shipping `EndAction` lifecycle.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` no longer names a live
    /// action, or when re-queuing an interrupted action fails because the
    /// handle went away in the meantime.
    ///
    /// # Panics
    ///
    /// Never in practice: the re-queue path re-looks-up the handle that was
    /// just validated a few statements earlier.
    pub fn end_action(
        &mut self,
        handle: ActionHandle,
        method: ActionEndMethod,
    ) -> Result<(), InvalidActionHandle> {
        debug_assert!(
            self.scopes.iter().all(|scope| {
                scope.action != Some(handle) && scope.exiting_action != Some(handle)
            })
        );

        match method {
            ActionEndMethod::Normal | ActionEndMethod::NormalLeaveAnimations => {
                let interruptable = {
                    let action = self.actions.get_mut(handle).ok_or(InvalidActionHandle)?;
                    Self::interrupt_action(action)
                };

                if interruptable {
                    let requeued = {
                        let action = self
                            .actions
                            .get_mut(handle)
                            .expect("validated action handle");
                        let queue_time = action.as_ref().queue_time;
                        action.as_mut().initialise(queue_time);
                        action.on_initialise();
                        action.as_ref().flags.contains(ActionFlags::REQUEUED)
                    };
                    if requeued {
                        self.actions
                            .get_mut(handle)
                            .expect("validated action handle")
                            .as_mut()
                            .flags
                            .remove(ActionFlags::REQUEUED);
                    } else {
                        self.queue.push_onto_queue(&self.actions, handle)?;
                    }
                } else if self
                    .actions
                    .get(handle)
                    .is_some_and(|action| action.as_ref().flags.contains(ActionFlags::REQUEUED))
                {
                    self.queue.remove(handle);
                }
            }
            ActionEndMethod::Failure => {
                let action = self.actions.get_mut(handle).ok_or(InvalidActionHandle)?;
                action.on_failure(ActionFailure::InvalidContext);
                action.as_mut().fail();
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn queued_actions(&self) -> impl ExactSizeIterator<Item = ActionHandle> + '_ {
        self.queue.iter()
    }

    #[must_use]
    pub fn is_action_pending(&self, user_token: u32) -> bool {
        self.queue.iter().any(|handle| {
            self.actions.get(handle).is_some_and(|action| {
                action.as_ref().status == ActionStatus::Pending
                    && action.as_ref().user_token == user_token
            })
        })
    }

    #[must_use]
    pub fn is_different(
        &self,
        fragment: super::FragmentId,
        fragment_tags: TagState,
        scopes: ScopeMask,
        driver: &impl ActionControllerDriver<A>,
    ) -> bool {
        let scopes = scopes & self.active_scopes;
        let mut installed_contexts = 0_u32;
        self.scopes
            .iter()
            .filter(|scope| scopes.contains_scope(scope.id))
            .any(|scope| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "scope context indices are bounded by MAX_SCOPES (32) and the \
                              shift is guarded by checked_shl regardless"
                )]
                let context_shift = scope.context.index() as u32;
                let context_bit = 1_u32.checked_shl(context_shift).unwrap_or(0);
                if !driver.scope_needs_install(scope, installed_contexts) {
                    return false;
                }
                installed_contexts |= context_bit;
                driver.is_scope_different(scope, fragment, fragment_tags)
            })
    }

    pub fn flush(&mut self, driver: &mut impl ActionControllerDriver<A>) {
        self.flush_with(ActionEndMethod::Normal, driver);
    }

    /// Ends every installed action and clears every scope.
    ///
    /// # Panics
    ///
    /// Panics if called while the controller is inside [`Self::update`], which
    /// Cry also forbids.
    pub fn flush_with(
        &mut self,
        method: ActionEndMethod,
        driver: &mut impl ActionControllerDriver<A>,
    ) {
        assert!(
            !self.flags.contains(ControllerFlags::IS_IN_UPDATE),
            "cannot flush a Mannequin controller during update"
        );

        let mut handles = std::mem::take(&mut self.scratch.handles);
        handles.clear();
        for scope in &self.scopes {
            driver.flush_scope(scope, method);
            if let Some(handle) = scope.playing_action()
                && !handles.contains(&handle)
            {
                handles.push(handle);
            }
        }
        for handle in handles.iter().copied() {
            if let Some(action) = self.actions.get_mut(handle) {
                match method {
                    ActionEndMethod::Normal | ActionEndMethod::NormalLeaveAnimations => {
                        Self::interrupt_action(action);
                    }
                    ActionEndMethod::Failure => {
                        action.on_failure(ActionFailure::InvalidContext);
                        action.as_mut().fail();
                    }
                }
            }
        }
        for scope in &mut self.scopes {
            scope.action = None;
            scope.exiting_action = None;
        }
        self.queue.clear(&mut self.actions);
        handles.clear();
        handles.extend(self.actions.iter().map(|(handle, _)| handle));
        for handle in handles.iter().copied() {
            self.actions.remove(handle);
        }
        self.scratch.handles = handles;
    }

    pub fn reset(&mut self, driver: &mut impl ActionControllerDriver<A>) {
        self.flush(driver);
        self.active_scopes = ScopeMask::empty();
    }

    /// Run the current `CActionController::Update` ordering.
    #[expect(
        clippy::iter_with_drain,
        reason = "the drained vectors are scratch buffers moved back into `self.scratch` \
                  afterwards, so `into_iter` would consume the reused allocation"
    )]
    pub fn update(
        &mut self,
        delta_time: f32,
        procedural_debug: ProceduralDebug,
        driver: &mut impl ActionControllerDriver<A>,
    ) {
        if self.flags.contains(ControllerFlags::PAUSED_UPDATE) {
            return;
        }

        let delta_time = delta_time * self.time_scale;
        self.flags.insert(ControllerFlags::IS_IN_UPDATE);

        if self
            .flags
            .contains(ControllerFlags::RESOLVE_QUEUE_BEFORE_UPDATE)
        {
            self.resolve_queue(delta_time, NORMAL_RESOLVE_ITERATIONS, driver);
            driver.snapshot_scope_state(&mut self.scopes);
            self.resolve_queue(delta_time, PRE_UPDATE_RESOLVE_ITERATIONS, driver);
            self.finish_last_pending();
        } else {
            driver.snapshot_scope_state(&mut self.scopes);
        }

        self.update_root_actions(delta_time, driver);

        if !self
            .flags
            .contains(ControllerFlags::RESOLVE_QUEUE_BEFORE_UPDATE)
        {
            self.resolve_queue(delta_time, NORMAL_RESOLVE_ITERATIONS, driver);
            self.finish_last_pending();
        }

        let mut removed = std::mem::take(&mut self.scratch.handles);
        removed.clear();
        self.queue.prune_into(&mut self.actions, &mut removed);
        for handle in removed.drain(..) {
            let _ = self.remove_action(handle);
        }
        self.scratch.handles = removed;

        let mut scope_events = std::mem::take(&mut self.scratch.scope_events);
        for index in 0..self.scopes.len() {
            if !self.active_scopes.contains_scope(self.scopes[index].id) {
                continue;
            }
            let playing_action = self.scopes[index].playing_action().and_then(|handle| {
                self.actions.get(handle).map(|action| {
                    (
                        handle,
                        action.as_ref().speed_bias,
                        action.as_ref().animation_weight,
                    )
                })
            });
            scope_events.clear();
            driver.update_scope(
                &mut self.scopes[index],
                playing_action,
                delta_time,
                procedural_debug,
                &mut scope_events,
            );
            for event in scope_events.drain(..) {
                self.handle_scope_event(event);
            }
        }
        self.scratch.scope_events = scope_events;
        driver.update_procedural_contexts(delta_time);
        self.flags.remove(ControllerFlags::IS_IN_UPDATE);
    }

    fn update_root_actions(
        &mut self,
        delta_time: f32,
        driver: &mut impl ActionControllerDriver<A>,
    ) {
        let mut handles = std::mem::take(&mut self.scratch.handles);
        handles.clear();
        handles.extend(self.scopes.iter().filter_map(ActionScope::playing_action));
        let mut updated = std::mem::take(&mut self.scratch.auxiliary_handles);
        updated.clear();

        for handle in handles.iter().copied() {
            if updated.contains(&handle) {
                continue;
            }
            let Some(action) = self.actions.get_mut(handle) else {
                continue;
            };
            let root_scope = action.as_ref().root_scope;
            if !root_scope.is_some_and(|root| {
                self.scopes
                    .get(root.index())
                    .is_some_and(|scope| scope.playing_action() == Some(handle))
            }) {
                continue;
            }

            let status = action.as_mut().update_from_controller(delta_time);
            driver.on_action_updated(handle, action, status);
            updated.push(handle);
        }
        self.scratch.handles = handles;
        self.scratch.auxiliary_handles = updated;
    }

    fn resolve_queue(
        &mut self,
        delta_time: f32,
        max_iterations: usize,
        driver: &mut impl ActionControllerDriver<A>,
    ) {
        let mut ticked = std::mem::take(&mut self.scratch.ticked_actions);
        ticked.clear();
        for _ in 0..max_iterations {
            if !self.resolve_action_installations(delta_time, &mut ticked, driver) {
                break;
            }
        }
        self.scratch.ticked_actions = ticked;
    }

    #[expect(
        clippy::iter_with_drain,
        reason = "the drained vectors are scratch buffers moved back into `self.scratch` \
                  afterwards, so `into_iter` would consume the reused allocation"
    )]
    fn resolve_action_installations(
        &mut self,
        delta_time: f32,
        ticked: &mut Vec<ActionHandle>,
        driver: &mut impl ActionControllerDriver<A>,
    ) -> bool {
        self.resolve_playing_actions(driver);

        let mut queued = std::mem::take(&mut self.scratch.handles);
        queued.clear();
        queued.extend(self.queue.iter());
        let mut scope_mask_accumulator = ScopeMask::empty();
        let mut triggered = std::mem::take(&mut self.scratch.triggered_actions);
        triggered.clear();
        let mut changed = false;

        for handle in queued.iter().copied() {
            let Some(action) = self.actions.get(handle) else {
                self.queue.remove(handle);
                continue;
            };
            let tags = FragmentTagState {
                global_tags: self.global_tags,
                fragment_tags: action.as_ref().fragment_tags,
            };
            let requested = (action.as_ref().forced_scope_mask
                | driver.query_scope_mask(action, tags, self.active_scopes))
                & self.active_scopes;
            let root_scope = requested
                .least_significant_scope()
                .or_else(|| ScopeId::new(0));

            let action = self
                .actions
                .get_mut(handle)
                .expect("validated action handle");
            action.as_mut().root_scope = root_scope;
            let is_requeued = action.as_ref().flags.contains(ActionFlags::REQUEUED);
            if !is_requeued && !ticked.contains(&handle) {
                action.as_mut().update_pending(delta_time);
                ticked.push(handle);
            } else if is_requeued {
                action.as_mut().flags.insert(ActionFlags::LAST_PENDING);
            }

            let state = action.as_ref();
            let remove = state.status == ActionStatus::Finished
                || state.flags.contains(ActionFlags::STOPPING)
                || (state.status != ActionStatus::Installed && is_requeued);
            if remove {
                self.queue.remove(handle);
                action
                    .as_mut()
                    .flags
                    .remove(ActionFlags::REQUEUED | ActionFlags::AUTO_REQUEUED);
                continue;
            }

            let can_have_no_scope = state.flags.contains(ActionFlags::SCOPELESS);
            let scopes_do_not_conflict = (scope_mask_accumulator & requested).is_empty();
            if (!requested.is_empty() || can_have_no_scope) && scopes_do_not_conflict {
                let reinstall = state.status == ActionStatus::Installed;
                scope_mask_accumulator |= requested;
                changed |= self.install_queued_action(
                    handle,
                    requested,
                    reinstall,
                    delta_time,
                    &mut triggered,
                    driver,
                );
            }
        }

        for (handle, reinstall, root_has_outro) in triggered.drain(..) {
            self.queue.remove(handle);
            let Some(action) = self.actions.get_mut(handle) else {
                continue;
            };
            if action.as_ref().status == ActionStatus::Installed {
                if reinstall {
                    action.as_mut().flags.remove(ActionFlags::REQUEUED);
                } else if !root_has_outro {
                    action.as_mut().enter();
                }
            }
            action.as_mut().end_installing();
        }

        let mut ending_actions = std::mem::take(&mut self.scratch.ending_actions);
        ending_actions.clear();
        driver.blend_off_actions(
            &mut self.actions,
            &mut self.scopes,
            self.active_scopes,
            delta_time,
            &mut ending_actions,
        );
        changed |= !ending_actions.is_empty();
        self.resolve_ending_actions(ending_actions.drain(..));
        self.scratch.handles = queued;
        self.scratch.triggered_actions = triggered;
        self.scratch.ending_actions = ending_actions;
        changed
    }

    /// Runs Cry's request/can-install/install handshake for one queued action
    /// whose requested scopes are still free this iteration.
    ///
    /// Returns whether the action was installed, which is what makes the
    /// enclosing resolve loop run another iteration.
    ///
    /// # Panics
    ///
    /// Never in practice: every re-lookup uses the handle the caller has just
    /// validated against the arena.
    fn install_queued_action(
        &mut self,
        handle: ActionHandle,
        requested: ScopeMask,
        reinstall: bool,
        delta_time: f32,
        triggered: &mut Vec<(ActionHandle, bool, bool)>,
        driver: &mut impl ActionControllerDriver<A>,
    ) -> bool {
        let expanded_requested = self.expand_overlapping_scopes(requested);
        self.request_blend_out(handle, expanded_requested);
        let can_install = self.can_install_by_priority(handle, expanded_requested);
        let action = self
            .actions
            .get_mut(handle)
            .expect("validated action handle");
        driver.request_install(handle, action, expanded_requested, &mut self.scopes);
        let comparisons = self.scope_install_comparisons(handle, expanded_requested);
        let action = self.actions.get(handle).expect("validated action handle");
        let readiness = can_install
            .then(|| {
                driver.can_install(
                    handle,
                    action,
                    requested,
                    expanded_requested,
                    &self.scopes,
                    &comparisons,
                    delta_time,
                )
            })
            .flatten();
        if readiness.is_some() {
            self.mark_transition_pending(&comparisons);
        }
        let Some(readiness) = readiness else {
            return false;
        };
        if !readiness.is_ready() {
            return false;
        }

        let time_remaining = readiness.time_remaining();
        let action = self
            .actions
            .get_mut(handle)
            .expect("validated action handle");
        let state = action.as_mut();
        state.begin_installing();
        if !reinstall {
            state.install();
        }
        let outcome = driver.install(handle, action, requested, &mut self.scopes, time_remaining);
        let state = action.as_mut();
        state.installed_scope_mask = outcome.installed_scopes;
        state.root_scope = Some(outcome.root_scope);
        state.flags.insert(ActionFlags::JUST_INSTALLED);
        for scope in &mut self.scopes {
            if outcome.cleared_scopes.contains_scope(scope.id) {
                scope.action = None;
            }
            if outcome.installed_scopes.contains_scope(scope.id) {
                scope.action = Some(handle);
            }
        }
        for event in outcome.scope_events {
            self.handle_scope_event(event);
        }
        self.resolve_ending_actions(outcome.ending_actions);
        triggered.push((handle, reinstall, outcome.root_has_outro_transition));
        true
    }

    fn resolve_ending_actions(&mut self, ending_actions: impl IntoIterator<Item = EndingAction>) {
        let mut unique = SmallVec::<[ActionHandle; MAX_SCOPES]>::new();
        for ending in ending_actions {
            if unique.contains(&ending.handle) {
                continue;
            }
            unique.push(ending.handle);

            for scope in &mut self.scopes {
                if scope.action == Some(ending.handle) {
                    scope.action = None;
                }
                if scope.exiting_action == Some(ending.handle) {
                    scope.exiting_action = None;
                }
            }

            if ending.transition_out {
                if let Some(action) = self.actions.get_mut(ending.handle) {
                    action.as_mut().transition_out_started();
                }
                if let Some(root) = self.scopes.get_mut(ending.root_scope.index()) {
                    root.exiting_action = Some(ending.handle);
                }
            } else {
                let _ = self.end_action(ending.handle, ActionEndMethod::Normal);
            }
        }
    }

    fn handle_scope_event(&mut self, event: ScopeEvent) {
        match event {
            ScopeEvent::ActionMutation { action, mutation } => {
                if let Some(action) = self.actions.get_mut(action) {
                    mutation.apply_to(action.as_mut());
                }
            }
            ScopeEvent::SequenceFinished { scope, layer } => {
                let Some(handle) = self
                    .scopes
                    .get(scope.index())
                    .and_then(ActionScope::playing_action)
                else {
                    return;
                };
                if let Some(action) = self.actions.get_mut(handle) {
                    action.on_sequence_finished(layer, scope);
                }
            }
            ScopeEvent::ClipInstalled {
                scope, clip_type, ..
            } => {
                let Some(scope_state) = self.scopes.get(scope.index()) else {
                    return;
                };
                let current = scope_state.action;
                let exiting = scope_state.exiting_action;

                if clip_type != ClipType::TransitionOutro
                    && let Some(exiting) = exiting
                    && self.actions.get(exiting).is_some_and(|action| {
                        action
                            .as_ref()
                            .flags
                            .contains(ActionFlags::TRANSITIONING_OUT)
                    })
                {
                    self.scopes[scope.index()].exiting_action = None;
                    let _ = self.end_action(exiting, ActionEndMethod::Normal);
                    if let Some(current) = current
                        && let Some(action) = self.actions.get_mut(current)
                        && action.as_ref().status == ActionStatus::Installed
                    {
                        action.as_mut().enter();
                    }
                }

                let Some(current) = current else {
                    return;
                };
                let Some(action) = self.actions.get_mut(current) else {
                    return;
                };
                if action.as_ref().root_scope != Some(scope) {
                    return;
                }
                match clip_type {
                    ClipType::Transition
                        if !action.as_ref().flags.contains(ActionFlags::TRANSITIONING) =>
                    {
                        action.as_mut().transition_started();
                    }
                    ClipType::Normal
                        if !action
                            .as_ref()
                            .flags
                            .contains(ActionFlags::PLAYING_FRAGMENT) =>
                    {
                        action.as_mut().fragment_started();
                    }
                    ClipType::Normal | ClipType::Transition | ClipType::TransitionOutro => {}
                }
            }
        }
    }

    fn mark_transition_pending(&mut self, comparisons: &[ScopeInstallComparison]) {
        let mut marked = ArrayVec::<ActionHandle, MAX_SCOPES>::new();
        for comparison in comparisons {
            let Some(handle) = comparison.current else {
                continue;
            };
            if marked.contains(&handle) {
                continue;
            }
            if let Some(action) = self.actions.get_mut(handle) {
                action
                    .as_mut()
                    .flags
                    .insert(ActionFlags::TRANSITION_PENDING);
                marked.push(handle);
            }
        }
    }

    fn scope_install_comparisons(
        &self,
        candidate: ActionHandle,
        requested: ScopeMask,
    ) -> ArrayVec<ScopeInstallComparison, MAX_SCOPES> {
        let Some(candidate_action) = self.actions.get(candidate) else {
            return ArrayVec::new();
        };
        self.scopes
            .iter()
            .filter(|scope| requested.contains_scope(scope.id))
            .map(|scope| {
                let current = scope.action.or(scope.exiting_action);
                let is_requeue = scope.action == Some(candidate);
                let Some(current_handle) = current else {
                    return ScopeInstallComparison {
                        scope: scope.id,
                        current: None,
                        current_status: None,
                        priority: super::PriorityComparison::Higher,
                        is_requeue,
                        times_transition: true,
                    };
                };
                let Some(current_action) = self.actions.get(current_handle) else {
                    return ScopeInstallComparison {
                        scope: scope.id,
                        current,
                        current_status: None,
                        priority: super::PriorityComparison::Higher,
                        is_requeue,
                        times_transition: true,
                    };
                };
                if is_requeue && candidate_action.as_ref().root_scope != Some(scope.id) {
                    ScopeInstallComparison {
                        scope: scope.id,
                        current,
                        current_status: Some(current_action.as_ref().status),
                        priority: super::PriorityComparison::Higher,
                        is_requeue,
                        times_transition: true,
                    }
                } else {
                    let times_transition = current_action.as_ref().root_scope == Some(scope.id);
                    ScopeInstallComparison {
                        scope: scope.id,
                        current,
                        current_status: Some(current_action.as_ref().status),
                        priority: candidate_action
                            .do_compare_priority(current_action, candidate == current_handle),
                        is_requeue,
                        times_transition,
                    }
                }
            })
            .collect()
    }

    /// Cry's `ExpandOverlappingScopes`: an installed multi-scope action makes
    /// every one of its scopes participate in arbitration and replacement.
    fn expand_overlapping_scopes(&self, requested: ScopeMask) -> ScopeMask {
        self.scopes.iter().fold(requested, |expanded, scope| {
            let Some(handle) = scope.action else {
                return expanded;
            };
            let Some(action) = self.actions.get(handle) else {
                return expanded;
            };
            let state = action.as_ref();
            if state.status != ActionStatus::Exiting
                && !(state.installed_scope_mask & requested).is_empty()
            {
                expanded | state.installed_scope_mask
            } else {
                expanded
            }
        }) & self.active_scopes
    }

    /// Cry's `RequestInstall` calls `OnRequestBlendOut` on the root action in
    /// every affected scope before checking transition timing.
    fn request_blend_out(&mut self, candidate: ActionHandle, requested: ScopeMask) {
        let requests = {
            let Some(candidate_action) = self.actions.get(candidate) else {
                return;
            };
            let mut requests =
                ArrayVec::<(ActionHandle, super::PriorityComparison), MAX_SCOPES>::new();
            for scope in &self.scopes {
                if !requested.contains_scope(scope.id) {
                    continue;
                }
                let Some(current) = scope.action else {
                    continue;
                };
                let Some(current_action) = self.actions.get(current) else {
                    continue;
                };
                if current_action.as_ref().root_scope != Some(scope.id) {
                    continue;
                }
                let comparison =
                    candidate_action.do_compare_priority(current_action, candidate == current);
                if !requests.iter().any(|(handle, _)| *handle == current) {
                    requests.push((current, comparison));
                }
            }
            requests
        };

        for (handle, comparison) in requests {
            if let Some(action) = self.actions.get_mut(handle) {
                action.on_request_blend_out(comparison);
            }
        }
    }

    /// The priority/CanBlendOut portion of `CActionScope::CanInstall`. Fragment
    /// and transition timing remains with the runtime database driver.
    fn can_install_by_priority(&self, candidate: ActionHandle, requested: ScopeMask) -> bool {
        let Some(candidate_action) = self.actions.get(candidate) else {
            return false;
        };

        self.scopes.iter().all(|scope| {
            if !requested.contains_scope(scope.id) {
                return true;
            }
            let Some(current) = scope.action.or(scope.exiting_action) else {
                return true;
            };
            if current == candidate && scope.action == Some(candidate) {
                return true;
            }
            let Some(current_action) = self.actions.get(current) else {
                return true;
            };
            if current_action.as_ref().root_scope != Some(scope.id) {
                return true;
            }
            let comparison = candidate_action.do_compare_priority(current_action, false);
            current_action.as_ref().can_blend_out(comparison)
        })
    }

    fn resolve_playing_actions(&mut self, driver: &mut impl ActionControllerDriver<A>) {
        let mut handles = std::mem::take(&mut self.scratch.handles);
        handles.clear();
        handles.extend(self.scopes.iter().filter_map(ActionScope::playing_action));
        let mut visited = std::mem::take(&mut self.scratch.auxiliary_handles);
        visited.clear();

        for handle in handles.iter().copied() {
            if visited.contains(&handle) {
                continue;
            }
            let Some(action) = self.actions.get_mut(handle) else {
                continue;
            };
            if action
                .as_ref()
                .root_scope
                .is_none_or(|root| root.index() >= self.scopes.len())
            {
                continue;
            }
            if driver.on_resolve_action_installations(handle, action) {
                if !action.as_ref().flags.contains(ActionFlags::JUST_INSTALLED) {
                    action.as_mut().flags.insert(ActionFlags::AUTO_REQUEUED);
                }
                if !self.queue.contains(handle) {
                    let _ = self.queue.requeue(&mut self.actions, handle);
                }
            }
            visited.push(handle);
        }
        self.scratch.handles = handles;
        self.scratch.auxiliary_handles = visited;
    }

    fn finish_last_pending(&mut self) {
        let mut handles = std::mem::take(&mut self.scratch.handles);
        handles.clear();
        handles.extend(self.queue.iter());
        for handle in handles.iter().copied() {
            let Some(action) = self.actions.get(handle) else {
                self.queue.remove(handle);
                continue;
            };
            if action.as_ref().status == ActionStatus::Installed
                || !action.as_ref().flags.contains(ActionFlags::LAST_PENDING)
            {
                continue;
            }

            self.actions
                .get_mut(handle)
                .expect("validated action handle")
                .as_mut()
                .flags
                .remove(ActionFlags::LAST_PENDING);

            // Shipping EndAction may append a reinitialised action before the
            // LastPending pass erases the original vector slot. Removing the
            // stable handle first gives the same final queue without mutation-
            // sensitive indexing or a transient duplicate identity.
            self.queue.remove(handle);
            let _ = self.end_action(handle, ActionEndMethod::Normal);
        }
        self.scratch.handles = handles;
    }

    fn interrupt_action(action: &mut A) -> bool {
        if action.as_ref().flags.contains(ActionFlags::INSTALLING) {
            action.on_cancel_install();
            action.as_mut().status = ActionStatus::None;
        } else if !action.as_ref().flags.contains(ActionFlags::STARTED) {
            action.as_mut().status = ActionStatus::None;
        } else {
            action.on_exit();
            action.as_mut().exit();
        }

        action.as_ref().flags.contains(ActionFlags::INTERRUPTABLE)
    }
}

impl<A> MannequinParameterSource for ActionController<A> {
    fn parameter(&self, name: Crc32) -> Option<&bevy_math::Isometry3d> {
        self.parameters.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mannequin::{Action, FragmentId};

    struct Driver;

    impl ActionControllerDriver<Action> for Driver {
        fn query_scope_mask(
            &mut self,
            _action: &Action,
            _tags: FragmentTagState,
            active_scopes: ScopeMask,
        ) -> ScopeMask {
            active_scopes
        }

        fn can_install(
            &mut self,
            _handle: ActionHandle,
            _action: &Action,
            _requested_scopes: ScopeMask,
            _affected_scopes: ScopeMask,
            _scopes: &[ActionScope],
            _comparisons: &[ScopeInstallComparison],
            _delta_time: f32,
        ) -> Option<InstallReadiness> {
            Some(InstallReadiness::Ready {
                time_remaining: 0.0,
            })
        }

        fn install(
            &mut self,
            _handle: ActionHandle,
            _action: &mut Action,
            requested_scopes: ScopeMask,
            _scopes: &mut [ActionScope],
            _time_remaining: f32,
        ) -> InstallOutcome {
            InstallOutcome {
                installed_scopes: requested_scopes,
                cleared_scopes: ScopeMask::empty(),
                root_scope: requested_scopes.least_significant_scope().unwrap(),
                root_has_outro_transition: false,
                ending_actions: SmallVec::new(),
                scope_events: SmallVec::new(),
            }
        }
    }

    #[test]
    fn controller_installs_highest_priority_action_on_scope() {
        let scopes = [ActionScope::new(
            ScopeId::new(0).unwrap(),
            ScopeContextId::new(0).unwrap(),
            0,
            1,
        )];
        let mut controller = ActionController::new(scopes).unwrap();
        let fragment = FragmentId::new(0).unwrap();
        let low = controller.insert_action(Action::builder(fragment).priority(1).build());
        let high = controller.insert_action(Action::builder(fragment).priority(2).build());
        controller.queue(low, None).unwrap();
        controller.queue(high, None).unwrap();

        controller.update(1.0 / 60.0, ProceduralDebug::Disabled, &mut Driver);

        assert_eq!(controller.scopes()[0].action, Some(high));
        assert_eq!(
            controller.actions().get(high).unwrap().status,
            ActionStatus::Installed
        );
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "a paused controller must not advance the clock at all, so the active \
                  time has to stay bit-identical to its initial 0.0"
    )]
    fn paused_controller_does_not_tick_actions() {
        let scopes = [ActionScope::new(
            ScopeId::new(0).unwrap(),
            ScopeContextId::new(0).unwrap(),
            0,
            1,
        )];
        let mut controller = ActionController::new(scopes).unwrap();
        let handle = controller.insert_action(
            Action::builder(FragmentId::new(0).unwrap())
                .flags(ActionFlags::SCOPELESS)
                .build(),
        );
        controller.queue(handle, None).unwrap();
        let mut driver = Driver;
        controller.pause(&mut driver);
        controller.update(1.0, ProceduralDebug::Disabled, &mut driver);

        assert_eq!(controller.actions().get(handle).unwrap().active_time, 0.0);
    }
}
