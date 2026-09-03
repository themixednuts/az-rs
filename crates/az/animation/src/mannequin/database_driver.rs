//! Production action-controller driver backed by cooked Mannequin databases.
//!
//! This is the renderer-independent port of `CActionController::CanInstall`,
//! `CActionController::Install`, `CActionScope::CanInstall`, and
//! `CActionController::BlendOffActions`. Asset identity and runtime animation
//! keys are deliberately separate: a project supplies the zero-copy lookup
//! capabilities while this module owns selection, transition timing, scope
//! arbitration, and atomic installation.

use super::{
    Action, ActionArena, ActionControllerDriver, ActionDuration, ActionEndMethod, ActionFlags,
    ActionHandle, ActionScope, ActionStatus, AnimationDatabase, AnimationLane, AnimationMetadata,
    AnimationPlayback, ControllerDefinition, ControllerFlags, EndingAction, FragmentData,
    FragmentFlags, FragmentOptionIndex, FragmentQuery, FragmentSelection, FragmentTagState,
    InstallOutcome, InstallReadiness, MAX_SCOPES, MannequinAction, MannequinDatabaseDefinition,
    OptionIndex, PriorityComparison, ProceduralDebug, ProceduralInstallMode, ProceduralPlayback,
    ResumeFlags, ScopeContextId, ScopeEvent, ScopeId, ScopeInstallComparison, ScopeMask,
    ScopeRuntime, TagState, TransitionFlags,
};
use arrayvec::ArrayVec;
use smallvec::SmallVec;

/// Supplies the cooked database and its packed tag definition for a scope
/// context. Scope context assignment remains a project/runtime concern.
pub trait MannequinDatabaseProvider<K, P> {
    type Definition: MannequinDatabaseDefinition;

    fn database(&self, context: ScopeContextId) -> Option<&AnimationDatabase<K, P>>;

    fn definition(&self, context: ScopeContextId) -> Option<&Self::Definition>;
}

/// Resolves an authored/cooked animation identity into a hot runtime key.
///
/// `AnimationMetadata` is evaluated on the source key while the database is
/// assembled; `resolve_animation` then consumes that assembled data without
/// introducing a second selection path.
pub trait RuntimeAnimationResolver<K, T>: AnimationMetadata<K> {
    fn resolve_animation(&self, animation: &K) -> Option<T>;
}

impl<K, T, F, M> RuntimeAnimationResolver<K, T> for (F, M)
where
    F: Fn(&K) -> Option<T>,
    M: AnimationMetadata<K>,
{
    fn resolve_animation(&self, animation: &K) -> Option<T> {
        self.0(animation)
    }
}

impl<K, T, F, M> AnimationMetadata<K> for (F, M)
where
    F: Fn(&K) -> Option<T>,
    M: AnimationMetadata<K>,
{
    fn duration(&self, animation: &K) -> Option<f32> {
        self.1.duration(animation)
    }

    fn is_variable_length(&self, animation: &K) -> bool {
        self.1.is_variable_length(animation)
    }
}

/// Random source used to select among matching fragment options.
pub trait MannequinRandom {
    fn next_u32(&mut self) -> u32;
}

impl<F> MannequinRandom for F
where
    F: FnMut() -> u32,
{
    fn next_u32(&mut self) -> u32 {
        self()
    }
}

/// One scope's resolved fragment installation: the scope, the fragment data it
/// selected (if any), and whether the scope is the action's first.
type ScopeFragmentInstallations<T, P> =
    ArrayVec<(ScopeId, Option<FragmentData<T, P>>, bool), MAX_SCOPES>;

/// Small deterministic generator suitable for authoritative animation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MannequinRandomState(u64);

impl MannequinRandomState {
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }
}

impl Default for MannequinRandomState {
    fn default() -> Self {
        Self::seeded(0)
    }
}

impl MannequinRandom for MannequinRandomState {
    fn next_u32(&mut self) -> u32 {
        // xorshift64*: stable, cheap, and sufficient for authored option choice.
        let mut value = self.0;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.0 = value;
        (value.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32
    }
}

#[derive(Debug)]
struct PreparedScope<T, P> {
    scope: ScopeId,
    tags: FragmentTagState,
    selection: Option<FragmentSelection>,
    data: Option<FragmentData<T, P>>,
    principle_context: bool,
}

#[derive(Debug)]
struct PreparedInstallation<T, P> {
    handle: ActionHandle,
    requested_scopes: ScopeMask,
    affected_scopes: ScopeMask,
    comparisons: ArrayVec<ScopeInstallComparison, MAX_SCOPES>,
    option: u32,
    was_random: bool,
    persistent_fragment: bool,
    timewarp_auto_reinstall: bool,
    scopes: ArrayVec<PreparedScope<T, P>, MAX_SCOPES>,
}

/// Direct database-backed controller implementation.
///
/// Construct this for one controller update while borrowing the project's
/// assets, runtime animation resolver, scope runtimes, and concrete
/// animation/procedural backend.
pub struct DatabaseActionControllerDriver<'a, K, T, P, D, M, R, B> {
    controller_definition: &'a ControllerDefinition,
    databases: &'a D,
    animations: &'a M,
    random: &'a mut R,
    backend: &'a mut B,
    scope_runtimes: &'a mut [ScopeRuntime<T, P>],
    global_tags: TagState,
    sub_context_tags: &'a [TagState],
    flags: ControllerFlags,
    prepared: Option<PreparedInstallation<T, P>>,
    _source: std::marker::PhantomData<fn() -> K>,
}

impl<'a, K, T, P, D, M, R, B> DatabaseActionControllerDriver<'a, K, T, P, D, M, R, B> {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the borrow set CActionController hands to its driver in one update"
    )]
    #[must_use]
    pub fn new(
        controller_definition: &'a ControllerDefinition,
        databases: &'a D,
        animations: &'a M,
        random: &'a mut R,
        backend: &'a mut B,
        scope_runtimes: &'a mut [ScopeRuntime<T, P>],
        global_tags: TagState,
        sub_context_tags: &'a [TagState],
        flags: ControllerFlags,
    ) -> Self {
        Self {
            controller_definition,
            databases,
            animations,
            random,
            backend,
            scope_runtimes,
            global_tags,
            sub_context_tags,
            flags,
            prepared: None,
            _source: std::marker::PhantomData,
        }
    }

    fn action_tags(&self, action: &Action, include_dynamic_sub_context: bool) -> FragmentTagState {
        let mut global_tags = self.global_tags;
        if let Some(sub_context_id) = action.sub_context {
            if include_dynamic_sub_context
                && let Some(dynamic_tags) = self.sub_context_tags.get(sub_context_id.index())
            {
                global_tags = self
                    .controller_definition
                    .global_tags
                    .union(global_tags, *dynamic_tags);
            }
            if let Some(sub_context) = self.controller_definition.sub_context(sub_context_id) {
                global_tags = self
                    .controller_definition
                    .global_tags
                    .union(global_tags, sub_context.additional_tags);
            }
        }
        FragmentTagState {
            global_tags,
            fragment_tags: action.fragment_tags,
        }
    }

    fn scope_tags(
        definition: &impl MannequinDatabaseDefinition,
        runtime: &ScopeRuntime<T, P>,
        mut tags: FragmentTagState,
    ) -> FragmentTagState
    where
        T: Clone,
        P: Clone,
    {
        tags.global_tags = definition
            .global_tag_definition()
            .union(tags.global_tags, runtime.additional_tags());
        tags
    }

    fn transition_time(
        &self,
        runtime: &ScopeRuntime<T, P>,
        context: ScopeContextId,
        fragment_to: Option<super::FragmentId>,
        tags: FragmentTagState,
    ) -> Option<f32>
    where
        T: Clone,
        P: Clone,
        D: MannequinDatabaseProvider<K, P>,
        B: AnimationPlayback<T>,
    {
        let Some(database) = self.databases.database(context) else {
            return Some(
                runtime.fragment_duration()
                    + runtime.transition_duration()
                    + runtime.transition_outro_duration()
                    - runtime.fragment_time(),
            );
        };
        let definition = self.databases.definition(context)?;
        let tags = Self::scope_tags(definition, runtime, tags);
        let mut query = runtime.blend_query(
            fragment_to,
            tags,
            false,
            true,
            self.flags.contains(ControllerFlags::NO_TRANSITIONS),
        );
        let loop_duration = self
            .backend
            .top_animation(AnimationLane::new(runtime.id(), runtime.base_layer()))
            .map(|animation| {
                query.normalized_time = animation.normalized_time;
                animation.expected_duration
            })
            .unwrap_or_default();
        let primary = database
            .transitions
            .find_best_blends(query, definition)
            .primary;
        let Some(selection) = primary else {
            return Some(runtime.calculate_fragment_time_remaining());
        };
        let blend = selection.blend;
        if !blend.flags.contains(TransitionFlags::CYCLIC) {
            return Some(runtime.calculate_fragment_time_remaining() + blend.start_time);
        }
        if !blend.flags.contains(TransitionFlags::CYCLE_LOCKED) {
            return Some(0.0);
        }

        let mut cycle_difference = blend.start_time - query.normalized_time;
        // The cycle-locked trigger fires the instant the source clip crosses
        // `blend.start_time`. Both operands below are deliberately taken from
        // different objects: `previous_normalized_time`/`normalized_time` are the
        // query's own frame bracket, while `start_time` is the blend's trigger
        // point. Naming them keeps that asymmetry explicit.
        let crossed_start_this_frame = query.previous_normalized_time < blend.start_time;
        let clip_looped_this_frame = query.previous_normalized_time > query.normalized_time;
        if crossed_start_this_frame || clip_looped_this_frame {
            cycle_difference = cycle_difference.max(0.0);
        } else if cycle_difference < 0.0 {
            cycle_difference += 1.0;
        }
        Some(loop_duration * cycle_difference)
    }

    fn prepare_installation<A>(
        &mut self,
        handle: ActionHandle,
        action: &A,
        requested_scopes: ScopeMask,
        affected_scopes: ScopeMask,
        scopes: &[ActionScope],
        comparisons: &[ScopeInstallComparison],
    ) -> Option<PreparedInstallation<T, P>>
    where
        A: MannequinAction,
        K: Clone,
        T: Clone,
        P: Clone,
        D: MannequinDatabaseProvider<K, P>,
        M: RuntimeAnimationResolver<K, T>,
        R: MannequinRandom,
    {
        let state = action.as_ref();
        let (option, was_random) = match state.option_index {
            OptionIndex::Random => (self.random.next_u32(), true),
            OptionIndex::Index(option) => (option, false),
        };
        let tags = self.action_tags(state, true);
        let fragment_flags = self
            .controller_definition
            .fragment(state.fragment_id)
            .map_or_else(FragmentFlags::empty, |fragment| fragment.flags);
        let persistent_fragment = fragment_flags.contains(FragmentFlags::PERSISTENT);
        let timewarp_auto_reinstall = state.flags.contains(ActionFlags::AUTO_REQUEUED)
            && fragment_flags.contains(FragmentFlags::TIMEWARP_AUTO_REINSTALL);
        let mut installed_contexts = 0_u32;
        let mut prepared_scopes = ArrayVec::new();

        for scope in scopes
            .iter()
            .filter(|scope| requested_scopes.contains_scope(scope.id))
        {
            let runtime = self.scope_runtimes.get(scope.id.index())?;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "scope context indices are bounded by MAX_SCOPES (32) and the \
                          shift is guarded by checked_shl regardless"
            )]
            let context_shift = scope.context.index() as u32;
            let context_bit = 1_u32.checked_shl(context_shift).unwrap_or(0);
            let principle_context = !runtime.additional_tags().is_empty()
                || context_bit == 0
                || installed_contexts & context_bit == 0;
            installed_contexts |= context_bit;
            let higher_priority = comparisons
                .iter()
                .find(|comparison| comparison.scope == scope.id)
                .is_none_or(|comparison| comparison.priority == PriorityComparison::Higher);

            let data = if let (Some(database), Some(definition)) = (
                self.databases.database(scope.context),
                self.databases.definition(scope.context),
            ) {
                let scope_tags = Self::scope_tags(definition, runtime, tags);
                let query = runtime.blend_query(
                    Some(state.fragment_id),
                    scope_tags,
                    higher_priority,
                    principle_context,
                    self.flags.contains(ControllerFlags::NO_TRANSITIONS),
                );
                let result = database.query(
                    query,
                    FragmentOptionIndex::new(option),
                    definition,
                    self.animations,
                );
                let mut mapped = result
                    .data
                    .try_map_animations(|source| {
                        self.animations.resolve_animation(&source).ok_or(())
                    })
                    .ok()?;
                mapped.apply_timing_overrides(
                    state.fragment_start_time_offset,
                    state.fragment_blend_time,
                );
                Some((mapped, result.selection))
            } else {
                None
            };
            let (data, selection) =
                data.map_or((None, None), |(data, selection)| (Some(data), selection));
            prepared_scopes.push(PreparedScope {
                scope: scope.id,
                tags,
                selection,
                data,
                principle_context,
            });
        }

        Some(PreparedInstallation {
            handle,
            requested_scopes,
            affected_scopes,
            comparisons: comparisons.iter().copied().collect(),
            option,
            was_random,
            persistent_fragment,
            timewarp_auto_reinstall,
            scopes: prepared_scopes,
        })
    }

    fn ending_actions(
        prepared: &PreparedInstallation<T, P>,
        scope_runtimes: &[ScopeRuntime<T, P>],
        existing_exiting: &[(ScopeId, ActionHandle)],
    ) -> SmallVec<[EndingAction; MAX_SCOPES]>
    where
        T: Clone,
        P: Clone,
    {
        let mut ending = SmallVec::<[EndingAction; MAX_SCOPES]>::new();
        for comparison in &prepared.comparisons {
            if !comparison.times_transition {
                continue;
            }
            let Some(handle) = comparison
                .current
                .filter(|handle| *handle != prepared.handle)
            else {
                continue;
            };
            let transition_out = prepared.requested_scopes.contains_scope(comparison.scope)
                && scope_runtimes
                    .get(comparison.scope.index())
                    .is_some_and(ScopeRuntime::has_outro_transition);
            ending.push(EndingAction {
                handle,
                root_scope: comparison.scope,
                transition_out,
            });
        }
        for &(scope, handle) in existing_exiting {
            if !ending.iter().any(|entry| entry.handle == handle) {
                ending.push(EndingAction {
                    handle,
                    root_scope: scope,
                    transition_out: false,
                });
            }
        }
        ending
    }

    /// Resolves the procedural install mode for a prepared installation,
    /// consuming the one-shot timewarp suppression flag when it is present.
    fn resolve_install_mode<A>(
        prepared: &PreparedInstallation<T, P>,
        action: &mut A,
    ) -> ProceduralInstallMode
    where
        A: MannequinAction,
    {
        let skip_timewarp_once = prepared.timewarp_auto_reinstall
            && action
                .as_ref()
                .flags
                .contains(ActionFlags::SKIP_TIMEWARP_AUTO_REINSTALL_ONCE);
        if skip_timewarp_once {
            action
                .as_mut()
                .flags
                .remove(ActionFlags::SKIP_TIMEWARP_AUTO_REINSTALL_ONCE);
        }
        if prepared.timewarp_auto_reinstall && !skip_timewarp_once {
            ProceduralInstallMode::TimeWarpReinstall
        } else {
            ProceduralInstallMode::Normal
        }
    }

    /// Applies and removes every `ActionMutation` emitted while procedural
    /// clips were entered synchronously, leaving all other scope events queued
    /// in their original order.
    fn drain_action_mutations<A>(
        scope_events: &mut SmallVec<[ScopeEvent; MAX_SCOPES]>,
        handle: ActionHandle,
        action: &mut A,
    ) where
        A: MannequinAction,
    {
        let mut event_index = 0;
        while event_index < scope_events.len() {
            if matches!(scope_events[event_index], ScopeEvent::ActionMutation { .. }) {
                let ScopeEvent::ActionMutation {
                    action: event_action,
                    mutation,
                } = scope_events.remove(event_index)
                else {
                    unreachable!();
                };
                debug_assert_eq!(event_action, handle);
                mutation.apply_to(action.as_mut());
            } else {
                event_index += 1;
            }
        }
    }

    /// Builds the blend-out fragment data for every installed scope of an
    /// action that is blending off.
    ///
    /// Returns `None` when any scope failed to resolve its animations; the
    /// caller must then queue nothing, matching Cry's all-or-nothing blend off.
    fn prepare_blend_off_scopes(
        &mut self,
        scopes: &[ActionScope],
        installed_scopes: ScopeMask,
        tags: FragmentTagState,
        priority: PriorityComparison,
    ) -> Option<ScopeFragmentInstallations<T, P>>
    where
        K: Clone,
        T: Clone,
        P: Clone,
        D: MannequinDatabaseProvider<K, P>,
        M: RuntimeAnimationResolver<K, T>,
        R: MannequinRandom,
    {
        let mut prepared = ArrayVec::<_, MAX_SCOPES>::new();
        let mut installed_contexts = 0_u32;
        let mut complete = true;
        for scope in scopes
            .iter()
            .filter(|scope| installed_scopes.contains_scope(scope.id))
        {
            let runtime = &self.scope_runtimes[scope.id.index()];
            #[expect(
                clippy::cast_possible_truncation,
                reason = "scope context indices are bounded by MAX_SCOPES (32) and the \
                          shift is guarded by checked_shl regardless"
            )]
            let context_shift = scope.context.index() as u32;
            let context_bit = 1_u32.checked_shl(context_shift).unwrap_or(0);
            let principle_context = !runtime.additional_tags().is_empty()
                || context_bit == 0
                || installed_contexts & context_bit == 0;
            installed_contexts |= context_bit;
            let data = if let (Some(database), Some(definition)) = (
                self.databases.database(scope.context),
                self.databases.definition(scope.context),
            ) {
                let scope_tags = Self::scope_tags(definition, runtime, tags);
                let query = runtime.blend_query(
                    None,
                    scope_tags,
                    priority == PriorityComparison::Higher,
                    principle_context,
                    self.flags.contains(ControllerFlags::NO_TRANSITIONS),
                );
                let result = database.query(
                    query,
                    FragmentOptionIndex::new(self.random.next_u32()),
                    definition,
                    self.animations,
                );
                let resolved = result
                    .data
                    .try_map_animations(|source| {
                        self.animations.resolve_animation(&source).ok_or(())
                    })
                    .ok();
                if resolved.is_none() {
                    complete = false;
                }
                resolved
            } else {
                None
            };
            prepared.push((scope.id, data, principle_context));
        }
        complete.then_some(prepared)
    }
}

impl<A, K, T, P, D, M, R, B> ActionControllerDriver<A>
    for DatabaseActionControllerDriver<'_, K, T, P, D, M, R, B>
where
    A: MannequinAction,
    K: Clone,
    T: Clone,
    P: Clone,
    D: MannequinDatabaseProvider<K, P>,
    M: RuntimeAnimationResolver<K, T>,
    R: MannequinRandom,
    B: AnimationPlayback<T> + ProceduralPlayback<P>,
{
    fn query_scope_mask(
        &mut self,
        action: &A,
        _tags: FragmentTagState,
        active_scopes: ScopeMask,
    ) -> ScopeMask {
        let tags = self.action_tags(action.as_ref(), true);
        self.controller_definition.scope_mask(
            action.as_ref().fragment_id,
            tags,
            action.as_ref().sub_context,
        ) & active_scopes
    }

    fn pause_scope(&mut self, scope: &ActionScope) {
        if let Some(runtime) = self.scope_runtimes.get_mut(scope.id.index()) {
            runtime.pause(self.backend);
        }
    }

    fn resume_scope(&mut self, scope: &ActionScope, flags: ResumeFlags) {
        if let Some(runtime) = self.scope_runtimes.get_mut(scope.id.index()) {
            runtime.resume(None, flags, self.backend);
        }
    }

    fn snapshot_scope_state(&mut self, scopes: &mut [ActionScope]) {
        for scope in scopes {
            if let Some(runtime) = self.scope_runtimes.get_mut(scope.id.index()) {
                runtime.snapshot_normalized_time();
            }
        }
    }

    fn flush_scope(&mut self, scope: &ActionScope, method: ActionEndMethod) {
        if let Some(runtime) = self.scope_runtimes.get_mut(scope.id.index()) {
            runtime.flush(method, self.backend);
        }
    }

    fn scope_needs_install(&self, scope: &ActionScope, installed_contexts: u32) -> bool {
        let Some(runtime) = self.scope_runtimes.get(scope.id.index()) else {
            return false;
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "scope context indices are bounded by MAX_SCOPES (32) and the \
                      shift is guarded by checked_shl regardless"
        )]
        let context_shift = scope.context.index() as u32;
        let context_bit = 1_u32.checked_shl(context_shift).unwrap_or(0);
        !runtime.additional_tags().is_empty()
            || context_bit == 0
            || installed_contexts & context_bit == 0
    }

    fn is_scope_different(
        &self,
        scope: &ActionScope,
        fragment: super::FragmentId,
        fragment_tags: TagState,
    ) -> bool {
        let Some(runtime) = self.scope_runtimes.get(scope.id.index()) else {
            return true;
        };
        if runtime.last_fragment() != Some(fragment) {
            return true;
        }
        let (Some(database), Some(definition)) = (
            self.databases.database(scope.context),
            self.databases.definition(scope.context),
        ) else {
            return true;
        };
        let tags = FragmentTagState {
            global_tags: definition
                .global_tag_definition()
                .union(self.global_tags, runtime.additional_tags()),
            fragment_tags,
        };
        let selection = database
            .fragments
            .best_entry(
                FragmentQuery {
                    fragment: Some(fragment),
                    tags,
                    required_global_tags: runtime.additional_tags(),
                    option: FragmentOptionIndex::new(0),
                },
                definition,
            )
            .map(|(_, selection)| selection.tag_set_index);
        selection
            != runtime
                .last_selection()
                .map(|selection| selection.tag_set_index)
    }

    fn query_duration(
        &mut self,
        _handle: ActionHandle,
        action: &mut A,
        requested_scopes: ScopeMask,
        scopes: &[ActionScope],
        comparisons: &[ScopeInstallComparison],
    ) -> Option<ActionDuration> {
        let (option, was_random) = match action.as_ref().option_index {
            OptionIndex::Random => (self.random.next_u32(), true),
            OptionIndex::Index(option) => (option, false),
        };
        if was_random {
            action.as_mut().option_index = OptionIndex::Index(option);
            action.as_mut().flags.insert(ActionFlags::WAS_RANDOM);
        }
        let tags = self.action_tags(action.as_ref(), true);

        for scope in scopes
            .iter()
            .filter(|scope| requested_scopes.contains_scope(scope.id))
        {
            let runtime = self.scope_runtimes.get(scope.id.index())?;
            let (Some(database), Some(definition)) = (
                self.databases.database(scope.context),
                self.databases.definition(scope.context),
            ) else {
                continue;
            };
            let higher_priority = comparisons
                .iter()
                .find(|comparison| comparison.scope == scope.id)
                .is_none_or(|comparison| comparison.priority == PriorityComparison::Higher);
            let query = runtime.blend_query(
                Some(action.as_ref().fragment_id),
                Self::scope_tags(definition, runtime, tags),
                higher_priority,
                true,
                self.flags.contains(ControllerFlags::NO_TRANSITIONS),
            );
            let mut data = database
                .query(
                    query,
                    FragmentOptionIndex::new(option),
                    definition,
                    self.animations,
                )
                .data;
            if !data
                .sequence_flags
                .contains(super::FragmentSequenceFlags::FRAGMENT)
            {
                continue;
            }
            data.apply_timing_overrides(
                action.as_ref().fragment_start_time_offset,
                action.as_ref().fragment_blend_time,
            );
            let mut duration = ActionDuration::default();
            for (part_type, part_duration) in data.part_types.into_iter().zip(data.durations) {
                if part_type == super::ClipType::Normal {
                    duration.fragment += part_duration;
                } else {
                    duration.transition += part_duration;
                }
            }
            return Some(duration);
        }
        None
    }

    fn can_install(
        &mut self,
        handle: ActionHandle,
        action: &A,
        requested_scopes: ScopeMask,
        affected_scopes: ScopeMask,
        scopes: &[ActionScope],
        comparisons: &[ScopeInstallComparison],
        delta_time: f32,
    ) -> Option<InstallReadiness> {
        self.prepared = None;
        let tags = self.action_tags(action.as_ref(), false);
        let mut time_remaining = 0.0_f32;

        for comparison in comparisons
            .iter()
            .filter(|comparison| comparison.times_transition)
        {
            let scope = scopes.get(comparison.scope.index())?;
            let runtime = self.scope_runtimes.get(comparison.scope.index())?;
            let immediate = comparison.current.is_none()
                || comparison.priority == PriorityComparison::Higher
                || matches!(
                    comparison.current_status,
                    Some(ActionStatus::Finished | ActionStatus::Exiting)
                );
            let scope_time = if immediate {
                0.0
            } else {
                if runtime.fragment_time() <= 0.0 {
                    return None;
                }
                self.transition_time(
                    runtime,
                    scope.context,
                    Some(action.as_ref().fragment_id),
                    tags,
                )?
            };
            time_remaining = time_remaining.max(scope_time);
        }

        let readiness = if delta_time >= time_remaining {
            let prepared = self.prepare_installation(
                handle,
                action,
                requested_scopes,
                affected_scopes,
                scopes,
                comparisons,
            )?;
            self.prepared = Some(prepared);
            InstallReadiness::Ready { time_remaining }
        } else {
            InstallReadiness::Waiting { time_remaining }
        };
        Some(readiness)
    }

    fn install(
        &mut self,
        handle: ActionHandle,
        action: &mut A,
        requested_scopes: ScopeMask,
        scopes: &mut [ActionScope],
        mut time_remaining: f32,
    ) -> InstallOutcome {
        let mut prepared = self
            .prepared
            .take()
            .filter(|prepared| {
                prepared.handle == handle && prepared.requested_scopes == requested_scopes
            })
            .expect("CanInstall must atomically prepare a matching installation");
        let reinstall = action.as_ref().status == ActionStatus::Installed;
        if prepared.was_random {
            action.as_mut().option_index = OptionIndex::Index(prepared.option);
            action.as_mut().flags.insert(ActionFlags::WAS_RANDOM);
        }
        let install_mode = Self::resolve_install_mode(&prepared, action);
        let enter_procedural_on_install = self
            .flags
            .contains(ControllerFlags::ENTER_PROCEDURAL_ON_INSTALL);
        let mut scope_events = SmallVec::new();

        let mut root_scope = requested_scopes
            .least_significant_scope()
            .or_else(|| ScopeId::new(0))
            .expect("zero is a valid scope id");
        let mut root_found = false;
        let mut root_has_outro_transition = false;
        let mut is_one_shot = true;
        let existing_exiting = scopes
            .iter()
            .filter(|scope| prepared.affected_scopes.contains_scope(scope.id))
            .filter_map(|scope| scope.exiting_action.map(|handle| (scope.id, handle)))
            .collect::<ArrayVec<_, MAX_SCOPES>>();

        for mut prepared_scope in prepared.scopes.drain(..) {
            let scope = &mut scopes[prepared_scope.scope.index()];
            let runtime = &mut self.scope_runtimes[prepared_scope.scope.index()];
            scope.exiting_action = None;
            let should_queue = !reinstall
                || runtime.is_one_shot()
                || !runtime
                    .is_same_selection(action.as_ref().fragment_id, prepared_scope.selection);
            if should_queue && let Some(data) = prepared_scope.data.take() {
                let waiting_on_root = !root_found;
                let installed = runtime.queue_fragment(
                    Some(action.as_ref().fragment_id),
                    prepared_scope.tags,
                    prepared_scope.selection,
                    data,
                    time_remaining,
                    handle,
                    action.as_ref().speed_bias,
                    action.as_ref().animation_weight,
                    action.as_ref().user_token,
                    waiting_on_root,
                    prepared.persistent_fragment,
                    prepared_scope.principle_context,
                );
                if enter_procedural_on_install {
                    runtime.enter_queued_procedurals(install_mode, self.backend, |event| {
                        scope_events.push(event);
                    });
                    Self::drain_action_mutations(&mut scope_events, handle, action);
                }
                if waiting_on_root && installed {
                    root_scope = prepared_scope.scope;
                    root_found = true;
                    time_remaining = runtime.fragment_start_time();
                    is_one_shot = runtime.is_one_shot();
                    root_has_outro_transition = runtime.has_outro_transition();
                }
            } else if !root_found && runtime.has_fragment() {
                root_scope = prepared_scope.scope;
                root_found = true;
                is_one_shot = runtime.is_one_shot();
                root_has_outro_transition = runtime.has_outro_transition();
            }
            scope.one_shot = runtime.is_one_shot();
            scope.fragment_time = runtime.fragment_time();
            scope.blend_out_duration = runtime.blend_out_duration();
        }

        action
            .as_mut()
            .flags
            .set(ActionFlags::FRAGMENT_IS_ONE_SHOT, is_one_shot);
        let ending_actions =
            Self::ending_actions(&prepared, self.scope_runtimes, &existing_exiting);
        InstallOutcome {
            installed_scopes: requested_scopes,
            cleared_scopes: prepared.affected_scopes,
            root_scope,
            root_has_outro_transition,
            ending_actions,
            scope_events,
        }
    }

    fn blend_off_actions(
        &mut self,
        actions: &mut ActionArena<A>,
        scopes: &mut [ActionScope],
        active_scopes: ScopeMask,
        delta_time: f32,
        ending_actions: &mut impl Extend<EndingAction>,
    ) {
        let roots = scopes
            .iter()
            .filter_map(|scope| {
                let handle = scope.action?;
                let action = actions.get(handle)?;
                (action.as_ref().root_scope == Some(scope.id)).then_some((scope.id, handle))
            })
            .collect::<ArrayVec<_, MAX_SCOPES>>();

        for (root_scope, handle) in roots {
            let Some(action) = actions.get(handle) else {
                continue;
            };
            let state = action.as_ref();
            let runtime = &self.scope_runtimes[root_scope.index()];
            if runtime.fragment_time() <= 0.0
                || state.flags.contains(ActionFlags::TRANSITION_PENDING)
            {
                continue;
            }
            let priority = if state.status == ActionStatus::Finished {
                PriorityComparison::Higher
            } else {
                PriorityComparison::Lower
            };
            if !state.can_blend_out(priority) {
                continue;
            }
            let tags = FragmentTagState {
                global_tags: self.global_tags,
                fragment_tags: TagState::EMPTY,
            };
            let Some(time_remaining) =
                self.transition_time(runtime, scopes[root_scope.index()].context, None, tags)
            else {
                continue;
            };
            if delta_time < time_remaining {
                continue;
            }

            let installed_scopes = state.installed_scope_mask & active_scopes;
            let Some(prepared) =
                self.prepare_blend_off_scopes(scopes, installed_scopes, tags, priority)
            else {
                continue;
            };

            for (scope_id, data, principle_context) in prepared {
                if let Some(data) = data {
                    self.scope_runtimes[scope_id.index()].queue_fragment(
                        None,
                        tags,
                        None,
                        data,
                        0.0,
                        handle,
                        state.speed_bias,
                        state.animation_weight,
                        0,
                        priority == PriorityComparison::Higher,
                        false,
                        principle_context,
                    );
                }
                scopes[scope_id.index()].action = None;
            }
            let transition_out = self.scope_runtimes[root_scope.index()].has_outro_transition();
            ending_actions.extend(std::iter::once(EndingAction {
                handle,
                root_scope,
                transition_out,
            }));
        }
    }

    fn update_scope(
        &mut self,
        scope: &mut ActionScope,
        playing_action: Option<(ActionHandle, f32, f32)>,
        delta_time: f32,
        procedural_debug: ProceduralDebug,
        events: &mut impl Extend<ScopeEvent>,
    ) {
        let Some(runtime) = self.scope_runtimes.get_mut(scope.id.index()) else {
            return;
        };
        runtime.update(
            delta_time,
            playing_action.map(|(_, speed, weight)| (speed, weight)),
            procedural_debug,
            self.backend,
            |event| events.extend(std::iter::once(event)),
        );
        scope.one_shot = runtime.is_one_shot();
        scope.fragment_time = runtime.fragment_time();
        scope.blend_out_duration = runtime.blend_out_duration();
    }
}
