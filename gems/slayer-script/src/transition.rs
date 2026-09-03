//! Native transition transaction and reentrant transition routing.

use std::sync::Arc;

use crate::{
    EventRuntimeId, FunctionAdapter, LayerId, ModuleAdapter, OperationInvocation,
    ParentSequenceChanged, ParentSequenceContext, RuntimeContext, RuntimeError, RuntimeExecutor,
    SequenceActionMask, SequenceChanged, SequenceId, SequencePhase, SequenceRuntimeId,
    SequenceTransitionRuntime, TransitionGuard, TransitionOutcome, TransitionRequest,
    runtime::{CallbackRegistrationTarget, PendingTransition},
    sequence::{DEFAULT_OUTGOING_TRANSITION_SECONDS, INFINITE_SEQUENCE_DURATION_SECONDS},
};

/// The sequence a layer is leaving, paired with its live transition record.
type OutgoingRecord = (SequenceId, SequenceRuntimeId);

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Runs one native `Trans` call at its exact call site.
    ///
    /// Guarded normal non-null calls execute the complete target/lifecycle/
    /// counter preflight immediately and then replace the single pending slot.
    /// Draining that slot calls this entry point again, matching native's
    /// second preflight rather than treating the target as prevalidated.
    pub(crate) fn request_transition(
        &mut self,
        layer_id: LayerId,
        request: TransitionRequest,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<TransitionOutcome, RuntimeError<M::Error, F::Error>> {
        self.ensure_healthy()?;
        let layer_index = layer_id.index();
        debug_assert_eq!(
            callback_target_layer_index(callback_registration_target),
            layer_index,
            "callback root must belong to the transitioned layer"
        );
        let Some(layer) = self.layers.get(layer_index) else {
            return Err(RuntimeError::UnknownLayer { layer: layer_id });
        };
        if let Some(next) = request.next()
            && self.program.sequence(next).is_none()
        {
            return Err(RuntimeError::UnknownSequence { sequence: next });
        }
        if let Some(next) = request.next()
            && !self
                .program
                .layer(layer_id)
                .expect("validated runtime layer must have a definition")
                .allows_sequence(next)
        {
            return Err(RuntimeError::SequenceNotBoundToLayer {
                layer: layer_id,
                sequence: next,
            });
        }
        let current = layer.current();
        if let Some(outcome) = self.preflight_transition(layer_id, current, request)? {
            return Ok(outcome);
        }
        let force = self.honors_force(current, request.next(), request.is_forced());
        if callback_guard_layer == Some(layer_id) && request.next().is_some() && !force {
            self.layer_driver[layer_index].pending_transition = Some(PendingTransition {
                request: TransitionRequest::new(
                    request.next(),
                    request.transition_frames(),
                    request.initial_time_frames(),
                    false,
                ),
            });
            return Ok(TransitionOutcome::Deferred);
        }
        self.apply_transition(
            layer_id,
            request,
            force,
            callback_guard_layer,
            callback_registration_target,
        )
    }

    // The native transition is one ordered transaction: splitting this method
    // would obscure the proved cleanup -> exit -> install -> notify -> enter
    // sequence and make reentrant pending-slot handling harder to audit.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_transition(
        &mut self,
        layer_id: LayerId,
        request: TransitionRequest,
        force: bool,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<TransitionOutcome, RuntimeError<M::Error, F::Error>> {
        let layer_index = layer_id.index();
        if self.layers.get(layer_index).is_none() {
            return Err(RuntimeError::UnknownLayer { layer: layer_id });
        }
        let transition_seconds = request.transition_frames().seconds();
        let initial_seconds = request.initial_time_frames().seconds();

        let mut entered_sequence = None;
        let outgoing = self.prepare_outgoing_record(
            layer_index,
            request.next(),
            force,
            callback_guard_layer,
            callback_registration_target,
        )?;
        // EXIT operations are intentionally unguarded outside the current-time
        // driver and may transition synchronously. Native reloads +0x58 after
        // they return; that live value owns `previous`, reuse, and outgoing.
        let previous = outgoing.map(|(sequence, _)| sequence);
        let reuse_infinite = previous == request.next()
            && previous.is_some_and(|sequence| {
                self.program.sequence(sequence).is_some_and(|definition| {
                    definition.duration().get().to_bits()
                        == INFINITE_SEQUENCE_DURATION_SECONDS.to_bits()
                })
            });
        if let Some((_, runtime_id)) = outgoing {
            self.finalize_outgoing_record(
                layer_index,
                runtime_id,
                transition_seconds,
                !reuse_infinite,
            );
        }
        // Native leaves the old sequence and scalar clocks installed while
        // stopping its callback trees and running its EXIT actions. Only the
        // subsequent install resets the incoming layer clocks.
        self.layers[layer_index].reset_time(initial_seconds);
        if reuse_infinite {
            let runtime_id = self.allocate_sequence_runtime_id()?;
            if let Some(index) = self.layers[layer_index]
                .records
                .iter()
                .rposition(|record| !record.exiting && Some(record.sequence) == previous)
            {
                let mut record = self.layers[layer_index].records.remove(index);
                let previous_runtime_id = record.runtime_id;
                record.runtime_id = runtime_id;
                record.exiting = false;
                record.remove = false;
                self.layers[layer_index].records.push(record);
                if let Some(callbacks) = self.layer_driver[layer_index]
                    .auxiliary_callback_roots
                    .remove(&previous_runtime_id)
                {
                    self.layer_driver[layer_index]
                        .auxiliary_callback_roots
                        .insert(runtime_id, callbacks);
                }
            } else if let Some(next) = request.next() {
                self.layers[layer_index]
                    .records
                    .push(SequenceTransitionRuntime::incoming(
                        runtime_id,
                        next,
                        initial_seconds,
                        0.0,
                    ));
            }
            entered_sequence = request.next();
        } else if let Some(next) = request.next() {
            let incoming_duration = if transition_seconds < 0.0 {
                self.program
                    .sequence(next)
                    .expect("validated transition target must exist")
                    .default_incoming_transition_seconds()
            } else {
                transition_seconds.max(0.0)
            };
            let runtime_id = self.allocate_sequence_runtime_id()?;
            self.layers[layer_index]
                .records
                .push(SequenceTransitionRuntime::incoming(
                    runtime_id,
                    next,
                    initial_seconds,
                    incoming_duration,
                ));
            entered_sequence = Some(next);
        }
        if request.next().is_none() && is_zero(request.transition_frames().get()) {
            self.remove_zero_duration_exiting_records(layer_index);
        }
        if self.layers[layer_index].kind() == crate::LayerKind::Auxiliary {
            self.layers[layer_index].projected_auxiliary_sequence = request.next();
            self.layers[layer_index].projected_auxiliary_runtime_id =
                request.next().and_then(|next| {
                    self.layers[layer_index]
                        .records
                        .iter()
                        .rfind(|record| !record.exiting && record.sequence == next)
                        .map(|record| record.runtime_id)
                });
        }
        self.layer_driver[layer_index].mutation_counter = self.layer_driver[layer_index]
            .mutation_counter
            .wrapping_add(1);
        self.materialize_current_event_tracks(
            layer_index,
            previous,
            transition_seconds,
            initial_seconds,
        )?;

        let changed = SequenceChanged {
            layer: layer_id,
            previous,
            current: request.next(),
        };
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, callback_guard_layer, false)
                .with_callback_registration_target(callback_registration_target);
            let result = functions.on_sequence_changed(changed, modules, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.function_failure(error);
        }
        let parent = request
            .next()
            .map_or_else(ParentSequenceContext::default, |next| {
                self.program
                    .sequence(next)
                    .expect("validated current sequence must exist")
                    .parent_context()
            });
        let parent_changed = ParentSequenceChanged {
            parent: parent.parent,
            resolved_value_words: parent.resolved_value_words,
            transition_frames: request.transition_frames().get(),
            initial_time_frames: request.initial_time_frames().get(),
            state_words: parent.state_words,
            layer: layer_id,
        };
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, callback_guard_layer, false)
                .with_callback_registration_target(callback_registration_target);
            let result =
                modules.dispatch_parent_sequence_changed(parent_changed, functions, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.module_failure(error);
        }
        if !force && let Some(next) = entered_sequence {
            self.execute_phase(
                layer_id,
                next,
                SequencePhase::Enter,
                callback_guard_layer,
                callback_registration_target,
            )?;
        }
        if !force {
            let (previous_seconds, current_seconds, force_exit) = {
                let layer = &self.layers[layer_index];
                (
                    layer.previous_time_seconds,
                    layer.current_time_seconds,
                    layer.wrapped || layer.reached_end,
                )
            };
            self.dispatch_active_sequence_callbacks(
                callback_registration_target,
                previous_seconds,
                current_seconds,
                force_exit,
                0.0,
                callback_guard_layer,
            )?;
        }
        Ok(TransitionOutcome::Applied(changed))
    }

    fn prepare_outgoing_record(
        &mut self,
        layer_index: usize,
        next: Option<SequenceId>,
        force: bool,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<Option<OutgoingRecord>, RuntimeError<M::Error, F::Error>> {
        if self.layers[layer_index].current().is_none() {
            return Ok(None);
        }
        if !force {
            let mark_stopped = self.layers[layer_index].current() != next;
            self.stop_active_sequence_callbacks(
                callback_registration_target,
                mark_stopped,
                callback_guard_layer,
            )?;
            self.stop_current_event_callbacks(layer_index, mark_stopped, callback_guard_layer)?;
            let Some(sequence) = self.layers[layer_index].current() else {
                return Ok(None);
            };
            self.execute_phase(
                self.layers[layer_index].id,
                sequence,
                SequencePhase::Exit,
                callback_guard_layer,
                callback_registration_target,
            )?;
        }

        let Some(sequence) = self.layers[layer_index].current() else {
            return Ok(None);
        };
        let runtime_id = self.layers[layer_index]
            .current_runtime_id()
            .or_else(|| {
                self.layers[layer_index]
                    .records
                    .iter()
                    .rfind(|record| !record.exiting && record.sequence == sequence)
                    .map(|record| record.runtime_id)
            })
            .expect("current sequence must have a transition record");
        Ok(Some((sequence, runtime_id)))
    }

    fn finalize_outgoing_record(
        &mut self,
        layer_index: usize,
        runtime_id: SequenceRuntimeId,
        transition_seconds: f32,
        mark_exiting: bool,
    ) {
        let Some(record_index) = self.layers[layer_index]
            .records
            .iter()
            .position(|record| record.runtime_id == runtime_id)
        else {
            return;
        };
        if !mark_exiting {
            return;
        }
        let outgoing_duration = if transition_seconds < 0.0 {
            DEFAULT_OUTGOING_TRANSITION_SECONDS
        } else {
            transition_seconds
        };
        let record = &mut self.layers[layer_index].records[record_index];
        record.exiting = true;
        record.transition_duration_seconds = outgoing_duration;
        record.transition_elapsed_seconds = 0.0;
    }

    fn execute_phase(
        &mut self,
        layer: LayerId,
        sequence: SequenceId,
        phase: SequencePhase,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let action_mask = match phase {
            SequencePhase::Exit => SequenceActionMask::EXIT,
            SequencePhase::Enter => SequenceActionMask::ENTER,
            SequencePhase::Update => SequenceActionMask::UPDATE,
        };
        self.execute_sequence_action_chain(
            layer,
            sequence,
            phase,
            action_mask,
            callback_guard_layer,
            callback_registration_target,
        )
    }

    pub(crate) fn execute_time_actions(
        &mut self,
        layer: LayerId,
        sequence: SequenceId,
        action_mask: SequenceActionMask,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.execute_sequence_action_chain(
            layer,
            sequence,
            SequencePhase::Update,
            action_mask,
            Some(layer),
            callback_registration_target,
        )
    }

    fn execute_sequence_action_chain(
        &mut self,
        layer: LayerId,
        sequence: SequenceId,
        phase: SequencePhase,
        action_mask: SequenceActionMask,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let program = Arc::clone(&self.program);
        let definition = program
            .sequence(sequence)
            .expect("validated action sequence must exist");
        if let Some(parent) = definition.parent_sequence() {
            let layer_index = layer.index();
            self.layer_driver[layer_index].callback_nesting_base = self.layer_driver[layer_index]
                .callback_nesting_base
                .wrapping_add(1_000_000);
            let result = self.execute_sequence_action_chain(
                layer,
                parent,
                phase,
                action_mask,
                callback_guard_layer,
                callback_registration_target,
            );
            self.layer_driver[layer_index].callback_nesting_base = self.layer_driver[layer_index]
                .callback_nesting_base
                .wrapping_sub(1_000_000);
            result?;
        }

        let program = Arc::clone(&self.program);
        let definition = program
            .sequence(sequence)
            .expect("validated action sequence must exist");
        for operation in definition.actions() {
            let (result, failure) = {
                let RuntimeExecutor {
                    state,
                    modules,
                    functions,
                } = self;
                let mut context =
                    RuntimeContext::<O, E, M, F>::new(state, callback_guard_layer, false)
                        .with_callback_registration_target(callback_registration_target);
                let result = functions.execute_operation(
                    OperationInvocation::new(layer, sequence, phase, action_mask, operation),
                    modules,
                    &mut context,
                );
                (result, context.take_failure())
            };
            if let Some(error) = failure {
                return Err(error);
            }
            if let Err(error) = result {
                return self.function_failure(error);
            }
        }

        Ok(())
    }

    pub(crate) fn apply_pending_transition(
        &mut self,
        layer_index: usize,
        _nesting: u8,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if let Some(pending) = self.layer_driver[layer_index].pending_transition.take() {
            self.request_transition(
                self.layers[layer_index].id,
                pending.request,
                None,
                CallbackRegistrationTarget::Layer { layer_index },
            )?;
        }
        Ok(())
    }

    fn honors_force(
        &self,
        previous: Option<SequenceId>,
        next: Option<SequenceId>,
        requested: bool,
    ) -> bool {
        requested
            && !previous
                .and_then(|id| self.program.sequence(id))
                .is_some_and(super::program::SequenceDefinition::requires_normal_transition)
            && !next
                .and_then(|id| self.program.sequence(id))
                .is_some_and(super::program::SequenceDefinition::requires_normal_transition)
    }

    fn preflight_transition(
        &mut self,
        layer: LayerId,
        current: Option<SequenceId>,
        request: TransitionRequest,
    ) -> Result<Option<TransitionOutcome>, RuntimeError<M::Error, F::Error>> {
        if let Some(next) = request.next()
            && self.guard_transition_target(layer, current, next)?
        {
            return Ok(Some(TransitionOutcome::BlockedByTarget { sequence: next }));
        }

        let blocked = match self.functions.transition_application_blocked() {
            Ok(blocked) => blocked,
            Err(error) => return self.function_failure(error),
        };
        if blocked {
            return Ok(Some(TransitionOutcome::BlockedByLifecycle));
        }

        let layer_index = layer.index();
        self.layers[layer_index].transition_count =
            self.layers[layer_index].transition_count.saturating_add(1);
        if self.layers[layer_index].transition_count > crate::MAX_TRANSITION_NESTING {
            return Ok(Some(TransitionOutcome::IgnoredNestingLimit));
        }
        Ok(None)
    }

    fn guard_transition_target(
        &mut self,
        layer: LayerId,
        current: Option<SequenceId>,
        next: SequenceId,
    ) -> Result<bool, RuntimeError<M::Error, F::Error>> {
        let blocked = match self.functions.blocks_transition_target(TransitionGuard {
            layer,
            current,
            next,
        }) {
            Ok(blocked) => blocked,
            Err(error) => return self.function_failure(error),
        };
        Ok(blocked)
    }

    fn allocate_sequence_runtime_id(
        &mut self,
    ) -> Result<SequenceRuntimeId, RuntimeError<M::Error, F::Error>> {
        let id = self.next_sequence_runtime_id;
        let Some(next) = self.next_sequence_runtime_id.checked_add(1) else {
            self.poisoned = true;
            return Err(RuntimeError::RuntimeIdExhausted);
        };
        self.next_sequence_runtime_id = next;
        Ok(SequenceRuntimeId::new(id))
    }

    pub(crate) fn allocate_event_runtime_id(
        &mut self,
    ) -> Result<EventRuntimeId, RuntimeError<M::Error, F::Error>> {
        let id = self.next_event_runtime_id;
        let Some(next) = self.next_event_runtime_id.checked_add(1) else {
            self.poisoned = true;
            return Err(RuntimeError::RuntimeIdExhausted);
        };
        self.next_event_runtime_id = next;
        Ok(EventRuntimeId::new(id))
    }
}

/// True for `+0.0` and `-0.0`: every bit below the sign bit is clear.
const fn is_zero(value: f32) -> bool {
    value.to_bits().trailing_zeros() >= 31
}

const fn callback_target_layer_index(target: CallbackRegistrationTarget) -> usize {
    match target {
        CallbackRegistrationTarget::Layer { layer_index }
        | CallbackRegistrationTarget::AuxiliaryRecord { layer_index, .. }
        | CallbackRegistrationTarget::CurrentSlot { layer_index, .. } => layer_index,
    }
}
