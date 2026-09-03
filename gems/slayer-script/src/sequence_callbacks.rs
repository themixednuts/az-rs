//! Sequence-owned callback roots, auxiliary roots, and deferred callback flush.

use crate::{
    EventCallbackPhase, FunctionAdapter, IntervalCallbackBinding, IntervalCallbackDefinition,
    IntervalCallbackInvocation, IntervalCallbackScope, ModuleAdapter, RuntimeContext, RuntimeError,
    RuntimeExecutor,
    runtime::{
        CallbackRegistrationTarget, DeferredCallbackPhases, DeferredCallbackWork,
        InflightCallbackObject, RetainedCallbackObject,
    },
};

const CALLBACK_PHASE_ORDER: [(DeferredCallbackPhases, EventCallbackPhase); 3] = [
    (DeferredCallbackPhases::EXIT, EventCallbackPhase::Exit),
    (DeferredCallbackPhases::ENTER, EventCallbackPhase::Enter),
    (DeferredCallbackPhases::UPDATE, EventCallbackPhase::Update),
];

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    pub(crate) fn register_sequence_callback(
        &mut self,
        definition: &IntervalCallbackDefinition<E>,
        target: CallbackRegistrationTarget,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer_index = callback_target_layer_index(target);
        let callback = self.initialize_interval_callback_instance(
            layer_index,
            definition,
            IntervalCallbackScope::Sequence,
            target,
        )?;
        let runtime_id = crate::CallbackRuntimeId::new(
            self.layer_driver[layer_index]
                .callback_nesting_base
                .wrapping_add(definition.authored_id().get()),
        );
        let duplicate_inflight = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .any(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            });
        if duplicate_inflight {
            return Ok(());
        }
        let object = RetainedCallbackObject {
            callback: crate::CurrentEventCallbackRuntime {
                instance_id: self.allocate_callback_instance_id()?,
                start_seconds: definition.start().get(),
                end_seconds: definition.end().get(),
                state: crate::CurrentEventCallbackState {
                    runtime_id,
                    active: false,
                    stopped: false,
                    deferred_exit: false,
                    may_defer: definition.may_defer(),
                },
                payload: Some(callback),
            },
        };
        match target {
            CallbackRegistrationTarget::Layer { layer_index } => {
                self.layer_driver[layer_index]
                    .retained_callbacks
                    .entry(runtime_id)
                    .or_insert(object);
            }
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id: sequence_runtime_id,
            } => {
                self.layer_driver[layer_index]
                    .auxiliary_callback_roots
                    .entry(sequence_runtime_id)
                    .or_default()
                    .entry(runtime_id)
                    .or_insert(object);
            }
            CallbackRegistrationTarget::CurrentSlot { .. } => {
                unreachable!("sequence callback registration cannot target a fixed slot")
            }
        }
        Ok(())
    }

    pub(crate) fn initialize_interval_callback_instance(
        &mut self,
        layer_index: usize,
        definition: &IntervalCallbackDefinition<E>,
        scope: IntervalCallbackScope,
        callback_registration_target: CallbackRegistrationTarget,
    ) -> Result<E, RuntimeError<M::Error, F::Error>> {
        let binding = IntervalCallbackBinding {
            layer: self.layers[layer_index].id,
            authored_id: definition.authored_id(),
            scope,
        };
        let mut callback = definition.payload().clone();
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false)
                .with_callback_registration_target(callback_registration_target);
            let result =
                functions.bind_interval_callback(binding, &mut callback, modules, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.function_failure(error);
        }
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false)
                .with_callback_registration_target(callback_registration_target);
            let result = functions.initialize_interval_callback(
                binding,
                &mut callback,
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
        Ok(callback)
    }

    /// Stops callback objects in the sequence root active at this exact call.
    pub(crate) fn stop_active_sequence_callbacks(
        &mut self,
        target: CallbackRegistrationTarget,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.stop_callback_root(target, mark_stopped, callback_guard_layer)
    }

    pub(crate) fn stop_auxiliary_callbacks(
        &mut self,
        layer_index: usize,
        runtime_id: crate::SequenceRuntimeId,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.stop_callback_root(
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id,
            },
            true,
            Some(self.layers[layer_index].id),
        )?;
        let callbacks = self.layer_driver[layer_index]
            .auxiliary_callback_roots
            .remove(&runtime_id);
        if let Some(callbacks) = callbacks {
            for (_, mut object) in callbacks {
                self.finalize_interval_callback_instance(
                    CallbackRegistrationTarget::AuxiliaryRecord {
                        layer_index,
                        runtime_id,
                    },
                    &mut object.callback,
                )?;
            }
        }
        Ok(())
    }

    fn stop_callback_root(
        &mut self,
        target: CallbackRegistrationTarget,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let runtime_ids = self.callback_root_runtime_ids(target);
        for runtime_id in runtime_ids {
            let Some(owned_inflight) = self.begin_inflight_callback(target, runtime_id) else {
                continue;
            };
            let result = (|| {
                let (state, _, _) = self
                    .inflight_callback_snapshot(target, runtime_id)
                    .expect("callback selected from a root must remain visible while in flight");
                if !state.stopped && state.active {
                    if state.may_defer {
                        self.inflight_callback_state_mut(target, runtime_id)
                            .expect("in-flight callback state must remain present")
                            .deferred_exit = true;
                    } else {
                        self.execute_callback_object(
                            target,
                            runtime_id,
                            EventCallbackPhase::Exit,
                            0.0,
                            callback_guard_layer,
                            true,
                        )?;
                        self.inflight_callback_state_mut(target, runtime_id)
                            .expect("in-flight callback state must remain present")
                            .active = mark_stopped;
                    }
                }
                self.inflight_callback_state_mut(target, runtime_id)
                    .expect("in-flight callback state must remain present")
                    .stopped = mark_stopped;
                Ok(())
            })();
            self.finish_inflight_callback(target, runtime_id, owned_inflight, true)?;
            result?;
        }
        Ok(())
    }

    pub(crate) fn flush_deferred_callback_queue(
        &mut self,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        for layer_index in 0..self.layer_driver.len() {
            self.flush_retained_callbacks(layer_index)?;
            while let Some(work) = self.layer_driver[layer_index].callback_queue.pop_front() {
                let target = CallbackRegistrationTarget::Layer { layer_index };
                let Some(owned_inflight) =
                    self.begin_inflight_callback(target, work.callback_runtime_id)
                else {
                    continue;
                };
                let result = (|| {
                    for (phase_mask, phase) in CALLBACK_PHASE_ORDER {
                        if !work.phases.contains(phase_mask) {
                            continue;
                        }
                        self.execute_callback_object(
                            target,
                            work.callback_runtime_id,
                            phase,
                            work.delta_seconds,
                            None,
                            false,
                        )?;
                        let state = self
                            .inflight_callback_state_mut(target, work.callback_runtime_id)
                            .expect("queued callback state must remain present");
                        apply_callback_phase(state, phase);
                    }
                    Ok(())
                })();
                let has_more_work = self.layer_driver[layer_index]
                    .callback_queue
                    .iter()
                    .any(|queued| queued.callback_runtime_id == work.callback_runtime_id);
                let retain = self
                    .inflight_callback_snapshot(target, work.callback_runtime_id)
                    .is_some_and(|(state, _, _)| !state.stopped || has_more_work);
                self.finish_inflight_callback(
                    target,
                    work.callback_runtime_id,
                    owned_inflight,
                    retain,
                )?;
                result?;
            }
        }
        Ok(())
    }

    pub(crate) fn destroy_stopped_retained_callbacks(
        &mut self,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        for layer_index in 0..self.layer_driver.len() {
            let stopped = self.layer_driver[layer_index]
                .retained_callbacks
                .iter()
                .filter_map(|(runtime_id, object)| {
                    object.callback.state.stopped.then_some(*runtime_id)
                })
                .collect::<Vec<_>>();
            for runtime_id in stopped {
                if let Some(mut object) = self.layer_driver[layer_index]
                    .retained_callbacks
                    .remove(&runtime_id)
                {
                    self.finalize_interval_callback_instance(
                        CallbackRegistrationTarget::Layer { layer_index },
                        &mut object.callback,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Dispatches the layer-owned retained callback map (`SequenceLayer +0x1a8`).
    ///
    /// The map contains runtime nodes migrated from outgoing fixed-slot trees;
    /// it is not a second authored callback table. Deferrable visits append one
    /// 12-byte-equivalent work record without coalescing across visits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_retained_callbacks(
        &mut self,
        layer_index: usize,
        previous_seconds: f32,
        current_seconds: f32,
        force_exit: bool,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.dispatch_callback_root(
            CallbackRegistrationTarget::Layer { layer_index },
            previous_seconds,
            current_seconds,
            force_exit,
            delta_seconds,
            callback_guard_layer,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_active_sequence_callbacks(
        &mut self,
        target: CallbackRegistrationTarget,
        previous_seconds: f32,
        current_seconds: f32,
        force_exit: bool,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.dispatch_callback_root(
            target,
            previous_seconds,
            current_seconds,
            force_exit,
            delta_seconds,
            callback_guard_layer,
        )
    }

    /// Dispatches the runtime-only callback tree owned by one auxiliary record.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_auxiliary_callbacks(
        &mut self,
        layer_index: usize,
        runtime_id: crate::SequenceRuntimeId,
        previous_seconds: f32,
        current_seconds: f32,
        force_exit: bool,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.dispatch_callback_root(
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id,
            },
            previous_seconds,
            current_seconds,
            force_exit,
            delta_seconds,
            Some(self.layers[layer_index].id),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_callback_root(
        &mut self,
        target: CallbackRegistrationTarget,
        previous_seconds: f32,
        current_seconds: f32,
        force_exit: bool,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer_index = callback_target_layer_index(target);
        if self.pending_guarded_transition(layer_index, callback_guard_layer) {
            return Ok(());
        }
        let runtime_ids = self.callback_root_runtime_ids(target);
        for runtime_id in runtime_ids {
            if self.pending_guarded_transition(layer_index, callback_guard_layer) {
                break;
            }
            let Some(owned_inflight) = self.begin_inflight_callback(target, runtime_id) else {
                continue;
            };
            let (state, start_seconds, end_seconds) = self
                .inflight_callback_snapshot(target, runtime_id)
                .expect("callback selected from a root must remain visible while in flight");
            if state.stopped {
                self.finish_inflight_callback(target, runtime_id, owned_inflight, true)?;
                continue;
            }
            let phases = Self::callback_phases(
                start_seconds,
                end_seconds,
                state,
                previous_seconds,
                current_seconds,
                force_exit,
            );
            let result = (|| {
                if state.may_defer {
                    Self::apply_deferred_phases(
                        self.inflight_callback_state_mut(target, runtime_id)
                            .expect("in-flight callback state must remain present"),
                        phases,
                    );
                    if !phases.is_empty() {
                        self.layer_driver[layer_index].callback_queue.push_back(
                            DeferredCallbackWork {
                                callback_runtime_id: runtime_id,
                                phases,
                                delta_seconds,
                            },
                        );
                    }
                } else {
                    for (phase_mask, phase) in CALLBACK_PHASE_ORDER {
                        if !phases.contains(phase_mask) {
                            continue;
                        }
                        self.execute_callback_object(
                            target,
                            runtime_id,
                            phase,
                            delta_seconds,
                            callback_guard_layer,
                            false,
                        )?;
                        let state = self
                            .inflight_callback_state_mut(target, runtime_id)
                            .expect("in-flight callback state must remain present");
                        apply_callback_phase(state, phase);
                    }
                }
                Ok(())
            })();
            self.finish_inflight_callback(target, runtime_id, owned_inflight, true)?;
            result?;
        }
        Ok(())
    }

    fn flush_retained_callbacks(
        &mut self,
        layer_index: usize,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let target = CallbackRegistrationTarget::Layer { layer_index };
        let runtime_ids = self.layer_driver[layer_index]
            .retained_callbacks
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            let Some(owned_inflight) = self.begin_inflight_callback(target, runtime_id) else {
                continue;
            };
            let result = (|| {
                let (state, _, _) = self
                    .inflight_callback_snapshot(target, runtime_id)
                    .expect("retained callback must remain visible while in flight");
                if state.deferred_exit && state.active {
                    self.execute_callback_object(
                        target,
                        runtime_id,
                        EventCallbackPhase::Exit,
                        0.0,
                        None,
                        false,
                    )?;
                    self.inflight_callback_state_mut(target, runtime_id)
                        .expect("retained callback state must remain present")
                        .active = false;
                }
                let state = self
                    .inflight_callback_state_mut(target, runtime_id)
                    .expect("retained callback state must remain present");
                if !state.stopped {
                    state.deferred_exit = false;
                    state.active = false;
                }
                Ok(())
            })();
            let retain = self
                .inflight_callback_snapshot(target, runtime_id)
                .is_some_and(|(state, _, _)| !state.stopped);
            self.finish_inflight_callback(target, runtime_id, owned_inflight, retain)?;
            result?;
        }
        Ok(())
    }

    fn pending_guarded_transition(
        &self,
        layer_index: usize,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> bool {
        callback_guard_layer == Some(self.layers[layer_index].id)
            && self.layer_driver[layer_index].pending_transition.is_some()
    }

    fn callback_root_runtime_ids(
        &self,
        target: CallbackRegistrationTarget,
    ) -> Vec<crate::CallbackRuntimeId> {
        let layer_index = callback_target_layer_index(target);
        let mut runtime_ids: Vec<crate::CallbackRuntimeId> = match target {
            CallbackRegistrationTarget::Layer { layer_index } => self.layer_driver[layer_index]
                .retained_callbacks
                .keys()
                .copied()
                .collect(),
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id,
            } => self.layer_driver[layer_index]
                .auxiliary_callback_roots
                .get(&runtime_id)
                .into_iter()
                .flat_map(|root| root.keys().copied())
                .collect(),
            CallbackRegistrationTarget::CurrentSlot { .. } => Vec::new(),
        };
        runtime_ids.extend(
            self.layer_driver[layer_index]
                .inflight_callbacks
                .iter()
                .filter(|inflight| inflight.target == target)
                .map(|inflight| inflight.object.callback.state.runtime_id),
        );
        runtime_ids.sort_unstable();
        runtime_ids.dedup();
        runtime_ids
    }

    /// Makes one runtime node visible through the in-flight overlay while its
    /// callback executes. Native leaves the node resident in the ordered map;
    /// the overlay provides the same Stop/duplicate-registration visibility
    /// without holding a Rust map borrow across reentrant adapter calls.
    fn begin_inflight_callback(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
    ) -> Option<bool> {
        let layer_index = callback_target_layer_index(target);
        if self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .any(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })
        {
            return Some(false);
        }
        let object = self.take_callback_object(target, runtime_id)?;
        self.layer_driver[layer_index]
            .inflight_callbacks
            .push(InflightCallbackObject { target, object });
        Some(true)
    }

    fn finish_inflight_callback(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
        owned_inflight: bool,
        retain: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if !owned_inflight {
            return Ok(());
        }
        let layer_index = callback_target_layer_index(target);
        let position = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .rposition(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })
            .expect("owned in-flight callback must remain on its layer stack");
        let inflight = self.layer_driver[layer_index]
            .inflight_callbacks
            .remove(position);
        if retain {
            self.reconcile_callback_object(target, runtime_id, inflight.object);
        } else {
            let mut object = inflight.object;
            self.finalize_interval_callback_instance(target, &mut object.callback)?;
        }
        Ok(())
    }

    pub(crate) fn finalize_interval_callback_instance(
        &mut self,
        target: CallbackRegistrationTarget,
        callback: &mut crate::CurrentEventCallbackRuntime<E>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let Some(mut payload) = callback.payload.take() else {
            self.poisoned = true;
            return Err(RuntimeError::UnsafeCallbackObjectReentry {
                callback: callback.state.runtime_id,
            });
        };
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false)
                .with_callback_registration_target(target);
            let result = functions.finalize_interval_callback(&mut payload, modules, &mut context);
            (result, context.take_failure())
        };
        callback.payload = Some(payload);
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.function_failure(error);
        }
        Ok(())
    }

    pub(crate) fn allocate_callback_instance_id(
        &mut self,
    ) -> Result<u64, RuntimeError<M::Error, F::Error>> {
        let id = self.next_callback_instance_id;
        let Some(next) = id.checked_add(1) else {
            self.poisoned = true;
            return Err(RuntimeError::RuntimeIdExhausted);
        };
        self.next_callback_instance_id = next;
        Ok(id)
    }

    fn inflight_callback_snapshot(
        &self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
    ) -> Option<(crate::CurrentEventCallbackState, f32, f32)> {
        let layer_index = callback_target_layer_index(target);
        let object = &self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .rfind(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })?
            .object;
        Some((
            object.callback.state,
            object.callback.start_seconds,
            object.callback.end_seconds,
        ))
    }

    fn inflight_callback_state_mut(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
    ) -> Option<&mut crate::CurrentEventCallbackState> {
        let layer_index = callback_target_layer_index(target);
        self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .rfind(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })
            .map(|inflight| &mut inflight.object.callback.state)
    }

    fn take_callback_object(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
    ) -> Option<RetainedCallbackObject<E>> {
        match target {
            CallbackRegistrationTarget::Layer { layer_index } => self.layer_driver[layer_index]
                .retained_callbacks
                .remove(&runtime_id),
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id: sequence_runtime_id,
            } => self.layer_driver[layer_index]
                .auxiliary_callback_roots
                .get_mut(&sequence_runtime_id)
                .and_then(|root| root.remove(&runtime_id)),
            CallbackRegistrationTarget::CurrentSlot { .. } => None,
        }
    }

    fn reconcile_callback_object(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
        object: RetainedCallbackObject<E>,
    ) {
        match target {
            CallbackRegistrationTarget::Layer { layer_index } => {
                self.layer_driver[layer_index]
                    .retained_callbacks
                    .entry(runtime_id)
                    .or_insert(object);
            }
            CallbackRegistrationTarget::AuxiliaryRecord {
                layer_index,
                runtime_id: sequence_runtime_id,
            } => {
                if let Some(root) = self.layer_driver[layer_index]
                    .auxiliary_callback_roots
                    .get_mut(&sequence_runtime_id)
                {
                    root.entry(runtime_id).or_insert(object);
                }
            }
            CallbackRegistrationTarget::CurrentSlot { .. } => {
                unreachable!("fixed-slot callbacks use their slot-owned reconciliation path")
            }
        }
    }

    fn callback_phases(
        start_seconds: f32,
        end_seconds: f32,
        state: crate::CurrentEventCallbackState,
        previous_seconds: f32,
        current_seconds: f32,
        force_exit: bool,
    ) -> DeferredCallbackPhases {
        let mut active = state.active;
        let mut phases = DeferredCallbackPhases::default();
        if active && (force_exit || end_seconds <= current_seconds) {
            phases.insert(DeferredCallbackPhases::EXIT);
            active = false;
        }
        if !active
            && ((start_seconds <= current_seconds && current_seconds < end_seconds)
                || (previous_seconds < start_seconds && end_seconds <= current_seconds))
        {
            phases.insert(DeferredCallbackPhases::ENTER);
            active = true;
        }
        if active {
            phases.insert(DeferredCallbackPhases::UPDATE);
        }
        phases
    }

    fn apply_deferred_phases(
        state: &mut crate::CurrentEventCallbackState,
        phases: DeferredCallbackPhases,
    ) {
        for (phase_mask, phase) in CALLBACK_PHASE_ORDER {
            if phases.contains(phase_mask) {
                apply_callback_phase(state, phase);
            }
        }
    }

    /// Runs one phase of one in-flight callback object.
    ///
    /// Every call site registers nested callbacks back into `target`, so the
    /// registration target is derived here rather than passed separately.
    fn execute_callback_object(
        &mut self,
        target: CallbackRegistrationTarget,
        runtime_id: crate::CallbackRuntimeId,
        phase: EventCallbackPhase,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
        synchronous_stop_exit: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer_index = callback_target_layer_index(target);
        let Some(mut callback) = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .rfind(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })
            .and_then(|inflight| inflight.object.callback.payload.take())
        else {
            self.poisoned = true;
            return Err(RuntimeError::UnsafeCallbackObjectReentry {
                callback: runtime_id,
            });
        };
        let result = self.invoke_current_event_callback(
            IntervalCallbackInvocation::new(runtime_id, phase, delta_seconds, &mut callback),
            callback_guard_layer,
            Some(target),
            synchronous_stop_exit,
        );
        if let Some(inflight) = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .rfind(|inflight| {
                inflight.target == target && inflight.object.callback.state.runtime_id == runtime_id
            })
        {
            inflight.object.callback.payload = Some(callback);
        }
        result
    }
}

const fn callback_target_layer_index(target: CallbackRegistrationTarget) -> usize {
    match target {
        CallbackRegistrationTarget::Layer { layer_index }
        | CallbackRegistrationTarget::AuxiliaryRecord { layer_index, .. }
        | CallbackRegistrationTarget::CurrentSlot { layer_index, .. } => layer_index,
    }
}

const fn apply_callback_phase(
    state: &mut crate::CurrentEventCallbackState,
    phase: EventCallbackPhase,
) {
    match phase {
        EventCallbackPhase::Exit => state.active = false,
        EventCallbackPhase::Enter => state.active = true,
        EventCallbackPhase::Update => {}
    }
}
