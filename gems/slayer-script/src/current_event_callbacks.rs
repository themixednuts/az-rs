//! Fixed current-slot callback traversal and migration into sequence roots.

use std::sync::Arc;

use crate::{
    CurrentEventRoute, CurrentEventTrackRuntime, EventCallbackPhase, FunctionAdapter,
    IntervalCallbackInvocation, ModuleAdapter, RuntimeError, RuntimeExecutor, SequenceId,
    runtime::{CallbackRegistrationTarget, RetainedCallbackObject},
};

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Stops active callback objects before the current sequence is replaced.
    pub(crate) fn stop_current_event_callbacks(
        &mut self,
        layer_index: usize,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let Some(sequence_id) = self.layers[layer_index].current() else {
            return Ok(());
        };
        let program = Arc::clone(&self.program);
        let sequence = program
            .sequence(sequence_id)
            .expect("validated current sequence must exist");
        let uses_inflight_dispatch = self.layer_driver[layer_index]
            .inflight_current_event_dispatch
            .as_ref()
            .is_some_and(|dispatch| dispatch.sequence == sequence_id);
        if uses_inflight_dispatch {
            let mut dispatch = self.layer_driver[layer_index]
                .inflight_current_event_dispatch
                .take()
                .expect("matched in-flight current-event dispatch must remain present");
            let mut inflight_slot = self.layer_driver[layer_index]
                .inflight_current_event_slot
                .take();
            let result = self.stop_current_event_vectors(
                sequence_id,
                layer_index,
                sequence,
                &mut dispatch.primary,
                &mut dispatch.payload,
                inflight_slot.as_mut(),
                mark_stopped,
                callback_guard_layer,
            );
            self.layer_driver[layer_index].inflight_current_event_dispatch = Some(dispatch);
            self.layer_driver[layer_index].inflight_current_event_slot = inflight_slot;
            return result;
        }
        let mut primary =
            std::mem::take(&mut self.layers[layer_index].current_primary_event_tracks);
        let mut payload =
            std::mem::take(&mut self.layers[layer_index].current_payload_event_tracks);
        let result = self.stop_current_event_vectors(
            sequence_id,
            layer_index,
            sequence,
            &mut primary,
            &mut payload,
            None,
            mark_stopped,
            callback_guard_layer,
        );
        self.layers[layer_index].current_primary_event_tracks = primary;
        self.layers[layer_index].current_payload_event_tracks = payload;
        result
    }

    /// Dispatches direct current-slot callbacks primary/payload by channel.
    pub(crate) fn dispatch_current_event_callbacks(
        &mut self,
        layer_index: usize,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let Some(sequence_id) = self.layers[layer_index].current() else {
            return Ok(());
        };
        let program = Arc::clone(&self.program);
        let sequence = program
            .sequence(sequence_id)
            .expect("validated callback sequence must exist");
        debug_assert!(
            self.layer_driver[layer_index]
                .inflight_current_event_dispatch
                .is_none(),
            "current-event dispatch cannot recursively enter the same layer"
        );
        self.layer_driver[layer_index].inflight_current_event_dispatch =
            Some(crate::runtime::InflightCurrentEventDispatch {
                sequence: sequence_id,
                primary: std::mem::take(&mut self.layers[layer_index].current_primary_event_tracks),
                payload: std::mem::take(&mut self.layers[layer_index].current_payload_event_tracks),
            });
        let mutation_before = self.layer_driver[layer_index].mutation_counter;
        let result = (|| {
            let group_count = self.layer_driver[layer_index]
                .inflight_current_event_dispatch
                .as_ref()
                .expect("current-event dispatch root must remain present")
                .primary
                .len();
            for group_index in 0..group_count {
                if self.begin_inflight_current_slot(
                    layer_index,
                    CurrentEventRoute::Primary,
                    group_index,
                ) {
                    let dispatch_result = self.dispatch_inflight_current_slot_callbacks(
                        layer_index,
                        sequence_id,
                        CurrentEventRoute::Primary,
                        delta_seconds,
                    );
                    self.finish_inflight_current_slot(layer_index);
                    dispatch_result?;
                }
                if let Some(group) = sequence.executable_payload_event_tracks().get(group_index)
                    && self.begin_inflight_current_slot(
                        layer_index,
                        CurrentEventRoute::Payload(group.owner()),
                        group_index,
                    )
                {
                    let dispatch_result = self.dispatch_inflight_current_slot_callbacks(
                        layer_index,
                        sequence_id,
                        CurrentEventRoute::Payload(group.owner()),
                        delta_seconds,
                    );
                    self.finish_inflight_current_slot(layer_index);
                    dispatch_result?;
                }
            }
            Ok(())
        })();
        let dispatch = self.layer_driver[layer_index]
            .inflight_current_event_dispatch
            .take()
            .expect("current-event dispatch root must be restored after traversal");
        debug_assert!(
            self.layer_driver[layer_index]
                .inflight_current_event_slot
                .is_none(),
            "current-event slot must be reconciled after traversal"
        );
        if self.layer_driver[layer_index].mutation_counter == mutation_before {
            self.layers[layer_index].current_primary_event_tracks = dispatch.primary;
            self.layers[layer_index].current_payload_event_tracks = dispatch.payload;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn stop_current_event_vectors(
        &mut self,
        sequence_id: SequenceId,
        layer_index: usize,
        sequence: &crate::SequenceDefinition<O, E>,
        primary: &mut [Option<CurrentEventTrackRuntime<E>>],
        payload: &mut [Option<CurrentEventTrackRuntime<E>>],
        mut inflight_slot: Option<&mut crate::runtime::InflightCurrentEventSlot<E>>,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        for group_index in 0..primary.len() {
            if inflight_slot.as_ref().is_some_and(|inflight| {
                inflight.slot.group_index == group_index
                    && inflight.route == CurrentEventRoute::Primary
            }) {
                self.stop_current_slot_callbacks(
                    sequence_id,
                    layer_index,
                    CurrentEventRoute::Primary,
                    &mut inflight_slot
                        .as_deref_mut()
                        .expect("matched in-flight primary slot must remain present")
                        .slot,
                    mark_stopped,
                    callback_guard_layer,
                )?;
            } else if let Some(slot) = primary[group_index].as_mut() {
                self.stop_current_slot_callbacks(
                    sequence_id,
                    layer_index,
                    CurrentEventRoute::Primary,
                    slot,
                    mark_stopped,
                    callback_guard_layer,
                )?;
            }
            let Some(group) = sequence.executable_payload_event_tracks().get(group_index) else {
                continue;
            };
            let route = CurrentEventRoute::Payload(group.owner());
            if inflight_slot.as_ref().is_some_and(|inflight| {
                inflight.slot.group_index == group_index && inflight.route == route
            }) {
                self.stop_current_slot_callbacks(
                    sequence_id,
                    layer_index,
                    route,
                    &mut inflight_slot
                        .as_deref_mut()
                        .expect("matched in-flight payload slot must remain present")
                        .slot,
                    mark_stopped,
                    callback_guard_layer,
                )?;
            } else if let Some(slot) = payload[group_index].as_mut() {
                self.stop_current_slot_callbacks(
                    sequence_id,
                    layer_index,
                    route,
                    slot,
                    mark_stopped,
                    callback_guard_layer,
                )?;
            }
        }
        Ok(())
    }

    fn stop_current_slot_callbacks(
        &mut self,
        _sequence_id: SequenceId,
        layer_index: usize,
        route: CurrentEventRoute,
        slot: &mut CurrentEventTrackRuntime<E>,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.stop_inflight_current_slot_callbacks(
            layer_index,
            route,
            slot.runtime_id,
            mark_stopped,
            callback_guard_layer,
        )?;
        for runtime_index in 0..slot.callbacks.len() {
            let Some(mut callback) = slot.callbacks[runtime_index].take() else {
                continue;
            };
            let mut state = callback.state;
            if state.active && state.may_defer {
                state.deferred_exit = true;
            } else if state.active {
                self.execute_current_callback(
                    layer_index,
                    &mut callback,
                    EventCallbackPhase::Exit,
                    0.0,
                    callback_guard_layer,
                    true,
                )?;
                state.active = mark_stopped;
            }
            state.stopped = mark_stopped;
            let target = CallbackRegistrationTarget::Layer { layer_index };
            let duplicate_inflight = self.layer_driver[layer_index]
                .inflight_callbacks
                .iter()
                .any(|inflight| {
                    inflight.target == target
                        && inflight.object.callback.state.runtime_id == state.runtime_id
                });
            if !duplicate_inflight {
                callback.state = state;
                self.layer_driver[layer_index]
                    .retained_callbacks
                    .entry(state.runtime_id)
                    .or_insert(RetainedCallbackObject { callback });
            }
        }
        Ok(())
    }

    fn stop_inflight_current_slot_callbacks(
        &mut self,
        layer_index: usize,
        route: CurrentEventRoute,
        event_runtime_id: crate::EventRuntimeId,
        mark_stopped: bool,
        callback_guard_layer: Option<crate::LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let source = CallbackRegistrationTarget::CurrentSlot {
            layer_index,
            event_runtime_id,
        };
        let instance_ids = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .filter(|inflight| inflight.target == source)
            .map(|inflight| inflight.object.callback.instance_id)
            .collect::<Vec<_>>();
        for instance_id in instance_ids {
            let state = self
                .inflight_current_callback_state(layer_index, instance_id)
                .expect("current-slot callback overlay must remain resident");
            if state.active && state.may_defer {
                self.inflight_current_callback_state_mut(layer_index, instance_id)
                    .expect("current-slot callback overlay must remain resident")
                    .deferred_exit = true;
            } else if state.active {
                self.execute_inflight_current_callback(
                    layer_index,
                    instance_id,
                    EventCallbackPhase::Exit,
                    0.0,
                    callback_guard_layer,
                    true,
                )?;
                self.inflight_current_callback_state_mut(layer_index, instance_id)
                    .expect("current-slot callback overlay must remain resident")
                    .active = mark_stopped;
            }
            self.inflight_current_callback_state_mut(layer_index, instance_id)
                .expect("current-slot callback overlay must remain resident")
                .stopped = mark_stopped;
            self.layer_driver[layer_index]
                .inflight_callbacks
                .iter_mut()
                .find(|inflight| inflight.object.callback.instance_id == instance_id)
                .expect("current-slot callback overlay must remain resident")
                .target = CallbackRegistrationTarget::Layer { layer_index };
        }
        let _ = route;
        Ok(())
    }

    fn begin_inflight_current_slot(
        &mut self,
        layer_index: usize,
        route: CurrentEventRoute,
        group_index: usize,
    ) -> bool {
        debug_assert!(
            self.layer_driver[layer_index]
                .inflight_current_event_slot
                .is_none(),
            "one current-event slot is visited at a time"
        );
        let dispatch = self.layer_driver[layer_index]
            .inflight_current_event_dispatch
            .as_mut()
            .expect("current-event dispatch root must remain present");
        let slot = match route {
            CurrentEventRoute::Primary => dispatch.primary[group_index].take(),
            CurrentEventRoute::Payload(_) => dispatch.payload[group_index].take(),
        };
        let Some(slot) = slot else {
            return false;
        };
        self.layer_driver[layer_index].inflight_current_event_slot =
            Some(crate::runtime::InflightCurrentEventSlot { route, slot });
        true
    }

    fn finish_inflight_current_slot(&mut self, layer_index: usize) {
        let Some(inflight) = self.layer_driver[layer_index]
            .inflight_current_event_slot
            .take()
        else {
            return;
        };
        let group_index = inflight.slot.group_index;
        let dispatch = self.layer_driver[layer_index]
            .inflight_current_event_dispatch
            .as_mut()
            .expect("current-event dispatch root must remain present");
        let destination = match inflight.route {
            CurrentEventRoute::Primary => &mut dispatch.primary[group_index],
            CurrentEventRoute::Payload(_) => &mut dispatch.payload[group_index],
        };
        if destination.is_none() {
            *destination = Some(inflight.slot);
        }
    }

    fn dispatch_inflight_current_slot_callbacks(
        &mut self,
        layer_index: usize,
        _sequence_id: SequenceId,
        _route: CurrentEventRoute,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let slot = &self.layer_driver[layer_index]
            .inflight_current_event_slot
            .as_ref()
            .expect("current-event slot must remain visible while in flight")
            .slot;
        if same_f32(
            slot.previous_playback_seconds,
            slot.current_playback_seconds,
        ) {
            return Ok(());
        }
        let callback_count = slot.callbacks.len();
        let event_runtime_id = slot.runtime_id;
        let previous_playback_seconds = slot.previous_playback_seconds;
        let current_playback_seconds = slot.current_playback_seconds;
        let force_exit = slot.current_playback_seconds < slot.previous_playback_seconds;
        for runtime_index in 0..callback_count {
            let Some(instance_id) =
                self.begin_inflight_current_callback(layer_index, event_runtime_id, runtime_index)
            else {
                continue;
            };
            let result = (|| {
                let callback = self
                    .inflight_current_callback(layer_index, instance_id)
                    .expect("current-slot callback overlay must remain resident");
                if callback.state.stopped {
                    return Ok(());
                }
                let start_seconds = callback.start_seconds;
                let end_seconds = callback.end_seconds;
                if callback.state.active && (force_exit || end_seconds <= current_playback_seconds)
                {
                    self.execute_inflight_current_callback(
                        layer_index,
                        instance_id,
                        EventCallbackPhase::Exit,
                        delta_seconds,
                        None,
                        false,
                    )?;
                    // Native EXIT observes active state and clears it after return.
                    self.inflight_current_callback_state_mut(layer_index, instance_id)
                        .expect("current-slot callback overlay must remain resident")
                        .active = false;
                }
                let active = self
                    .inflight_current_callback_state(layer_index, instance_id)
                    .expect("current-slot callback overlay must remain resident")
                    .active;
                if !active
                    && ((start_seconds <= current_playback_seconds
                        && current_playback_seconds < end_seconds)
                        || (previous_playback_seconds < start_seconds
                            && end_seconds <= current_playback_seconds))
                {
                    self.execute_inflight_current_callback(
                        layer_index,
                        instance_id,
                        EventCallbackPhase::Enter,
                        delta_seconds,
                        None,
                        false,
                    )?;
                    // Native ENTER observes inactive state and sets it after return.
                    self.inflight_current_callback_state_mut(layer_index, instance_id)
                        .expect("current-slot callback overlay must remain resident")
                        .active = true;
                }
                if self
                    .inflight_current_callback_state(layer_index, instance_id)
                    .expect("current-slot callback overlay must remain resident")
                    .active
                {
                    self.execute_inflight_current_callback(
                        layer_index,
                        instance_id,
                        EventCallbackPhase::Update,
                        delta_seconds,
                        None,
                        false,
                    )?;
                }
                Ok(())
            })();
            self.finish_inflight_current_callback(layer_index, runtime_index, instance_id)?;
            result?;
        }
        Ok(())
    }

    fn begin_inflight_current_callback(
        &mut self,
        layer_index: usize,
        event_runtime_id: crate::EventRuntimeId,
        runtime_index: usize,
    ) -> Option<u64> {
        let callback = self.layer_driver[layer_index]
            .inflight_current_event_slot
            .as_mut()
            .expect("current-event slot must remain visible while in flight")
            .slot
            .callbacks[runtime_index]
            .take()?;
        let instance_id = callback.instance_id;
        self.layer_driver[layer_index].inflight_callbacks.push(
            crate::runtime::InflightCallbackObject {
                target: CallbackRegistrationTarget::CurrentSlot {
                    layer_index,
                    event_runtime_id,
                },
                object: RetainedCallbackObject { callback },
            },
        );
        Some(instance_id)
    }

    fn finish_inflight_current_callback(
        &mut self,
        layer_index: usize,
        runtime_index: usize,
        instance_id: u64,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let position = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .position(|inflight| inflight.object.callback.instance_id == instance_id)
            .expect("owned current-slot callback overlay must remain resident");
        let inflight = self.layer_driver[layer_index]
            .inflight_callbacks
            .remove(position);
        match inflight.target {
            CallbackRegistrationTarget::CurrentSlot {
                event_runtime_id, ..
            } => {
                let slot = &mut self.layer_driver[layer_index]
                    .inflight_current_event_slot
                    .as_mut()
                    .expect("current-event slot must remain visible while in flight")
                    .slot;
                if slot.runtime_id == event_runtime_id {
                    slot.callbacks[runtime_index] = Some(inflight.object.callback);
                } else {
                    let mut callback = inflight.object.callback;
                    self.finalize_interval_callback_instance(
                        CallbackRegistrationTarget::CurrentSlot {
                            layer_index,
                            event_runtime_id,
                        },
                        &mut callback,
                    )?;
                }
            }
            CallbackRegistrationTarget::Layer { .. } => {
                let runtime_id = inflight.object.callback.state.runtime_id;
                if let std::collections::btree_map::Entry::Vacant(entry) = self.layer_driver
                    [layer_index]
                    .retained_callbacks
                    .entry(runtime_id)
                {
                    entry.insert(inflight.object);
                } else {
                    let mut callback = inflight.object.callback;
                    self.finalize_interval_callback_instance(
                        CallbackRegistrationTarget::Layer { layer_index },
                        &mut callback,
                    )?;
                }
            }
            CallbackRegistrationTarget::AuxiliaryRecord { .. } => {
                unreachable!("fixed-slot callbacks migrate only to their layer root")
            }
        }
        Ok(())
    }

    fn inflight_current_callback(
        &self,
        layer_index: usize,
        instance_id: u64,
    ) -> Option<&crate::CurrentEventCallbackRuntime<E>> {
        self.layer_driver[layer_index]
            .inflight_callbacks
            .iter()
            .find(|inflight| inflight.object.callback.instance_id == instance_id)
            .map(|inflight| &inflight.object.callback)
    }

    fn inflight_current_callback_state(
        &self,
        layer_index: usize,
        instance_id: u64,
    ) -> Option<crate::CurrentEventCallbackState> {
        self.inflight_current_callback(layer_index, instance_id)
            .map(|callback| callback.state)
    }

    fn inflight_current_callback_state_mut(
        &mut self,
        layer_index: usize,
        instance_id: u64,
    ) -> Option<&mut crate::CurrentEventCallbackState> {
        self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .find(|inflight| inflight.object.callback.instance_id == instance_id)
            .map(|inflight| &mut inflight.object.callback.state)
    }

    fn execute_inflight_current_callback(
        &mut self,
        layer_index: usize,
        instance_id: u64,
        phase: EventCallbackPhase,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
        synchronous_stop_exit: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let callback = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .find(|inflight| inflight.object.callback.instance_id == instance_id)
            .expect("current-slot callback overlay must remain resident");
        let runtime_id = callback.object.callback.state.runtime_id;
        let Some(mut payload) = callback.object.callback.payload.take() else {
            self.poisoned = true;
            return Err(RuntimeError::UnsafeCallbackObjectReentry {
                callback: runtime_id,
            });
        };
        let result = self.invoke_current_event_callback(
            IntervalCallbackInvocation::new(runtime_id, phase, delta_seconds, &mut payload),
            callback_guard_layer,
            Some(CallbackRegistrationTarget::Layer { layer_index }),
            synchronous_stop_exit,
        );
        if let Some(callback) = self.layer_driver[layer_index]
            .inflight_callbacks
            .iter_mut()
            .find(|inflight| inflight.object.callback.instance_id == instance_id)
        {
            callback.object.callback.payload = Some(payload);
        }
        result
    }

    fn execute_current_callback(
        &mut self,
        layer_index: usize,
        callback: &mut crate::CurrentEventCallbackRuntime<E>,
        phase: EventCallbackPhase,
        delta_seconds: f32,
        callback_guard_layer: Option<crate::LayerId>,
        synchronous_stop_exit: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let Some(mut payload) = callback.payload.take() else {
            self.poisoned = true;
            return Err(RuntimeError::UnsafeCallbackObjectReentry {
                callback: callback.state.runtime_id,
            });
        };
        let invocation = IntervalCallbackInvocation::new(
            callback.state.runtime_id,
            phase,
            delta_seconds,
            &mut payload,
        );
        let result = self.invoke_current_event_callback(
            invocation,
            callback_guard_layer,
            Some(CallbackRegistrationTarget::Layer { layer_index }),
            synchronous_stop_exit,
        );
        callback.payload = Some(payload);
        result
    }
}

fn same_f32(left: f32, right: f32) -> bool {
    left.partial_cmp(&right) == Some(std::cmp::Ordering::Equal)
}
