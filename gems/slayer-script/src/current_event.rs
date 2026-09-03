//! Fixed current-layer executable event slots (`SequenceLayer +0x78/+0x98`).
//!
//! These slots are deliberately separate from the fading event records embedded
//! in each `0x150` transition record. Native gives the two lanes different
//! ownership, advancement, and callback scheduling.

use std::sync::Arc;

use crate::{
    CurrentEventCallbackRuntime, CurrentEventCallbackState, CurrentEventHostExecution,
    CurrentEventRoute, CurrentEventStartRequest, CurrentEventStepRequest, CurrentEventStopRequest,
    CurrentEventTrackRuntime, CurrentEventUpdateRequest, EventIntervalDefinition,
    ExecutableEventChannel, FunctionAdapter, LayerKind, ModuleAdapter, RuntimeError,
    RuntimeExecutor,
};

/// The per-interval callback slots one fixed current-event slot owns.
type CurrentEventCallbackSlots<E> = Vec<Option<CurrentEventCallbackRuntime<E>>>;

/// One fixed current-event slot, absent while its group has no live interval.
type CurrentEventSlot<E> = Option<CurrentEventTrackRuntime<E>>;

/// What one aligned payload slot inherits from the primary slot it follows.
#[derive(Debug, Clone, Copy)]
struct PayloadAdvance {
    /// Index of the primary group this payload slot is aligned with.
    group_index: usize,
    /// Interval the primary slot settled on for this update.
    target_index: usize,
    /// Whether the primary slot moved to a different interval.
    target_changed: bool,
    /// Normalized playback position computed for the primary slot.
    normalized_playback: f32,
    /// Update delta already scaled by the primary interval's playback scale.
    delta_seconds: f32,
}

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Applies the current-layer external-step replacement once per update.
    pub(crate) fn current_event_step(
        &mut self,
        layer_index: usize,
        scaled_delta_seconds: f32,
    ) -> Result<f32, RuntimeError<M::Error, F::Error>> {
        if self.layers[layer_index].kind() != LayerKind::Normal {
            return Ok(scaled_delta_seconds);
        }
        let Some(sequence_id) = self.layers[layer_index].current() else {
            return Ok(scaled_delta_seconds);
        };
        let program = Arc::clone(&self.program);
        let sequence = program
            .sequence(sequence_id)
            .expect("validated current sequence must exist");
        let has_external = self.layers[layer_index]
            .current_primary_event_tracks
            .iter()
            .flatten()
            .any(|slot| {
                sequence.executable_event_tracks()[slot.group_index].intervals()
                    [slot.interval_index]
                    .is_externally_driven()
            });
        if !has_external {
            return Ok(scaled_delta_seconds);
        }
        if self.current_event_host_execution == CurrentEventHostExecution::Suppressed {
            return Ok(scaled_delta_seconds);
        }

        let route_key = self.layers[layer_index]
            .records
            .last()
            .expect("current sequence must own a transition record")
            .external_drive_route_key;
        let step = self.with_primary_current_event_host(|host| {
            host.replace_current_event_step(CurrentEventStepRequest {
                route_key,
                delta_seconds: scaled_delta_seconds,
            })
        })?;
        if !step.is_finite() {
            self.poisoned = true;
            return Err(RuntimeError::InvalidCurrentEventStep {
                layer: self.layers[layer_index].id,
            });
        }
        Ok(step)
    }

    /// Rebuilds the fixed current slots after a normal or forced transition.
    pub(crate) fn materialize_current_event_tracks(
        &mut self,
        layer_index: usize,
        previous_sequence: Option<crate::SequenceId>,
        transition_seconds: f32,
        initial_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if self.layers[layer_index].kind() != LayerKind::Normal {
            return Ok(());
        }
        let old_authored_group_count = previous_sequence.map_or(0, |sequence| {
            usize::try_from(
                self.program
                    .sequence(sequence)
                    .expect("validated previous sequence must exist")
                    .authored_primary_event_group_count()
                    .get(),
            )
            .expect("u32 authored group counts must fit usize")
        });
        let Some(sequence_id) = self.layers[layer_index].current() else {
            for group_index in 0..old_authored_group_count {
                self.stop_current_event(
                    CurrentEventRoute::Primary,
                    CurrentEventStopRequest {
                        channel: self.executable_event_channel(layer_index, group_index),
                        fade_seconds: transition_seconds,
                    },
                )?;
            }
            self.layers[layer_index]
                .current_primary_event_tracks
                .clear();
            self.layers[layer_index]
                .current_payload_event_tracks
                .clear();
            return Ok(());
        };

        let program = Arc::clone(&self.program);
        let sequence = program
            .sequence(sequence_id)
            .expect("validated materialized sequence must exist");
        let group_count = sequence.executable_event_tracks().len();
        let transition_group_count = old_authored_group_count.max(group_count);
        let mut primary = Vec::with_capacity(group_count);
        let mut payload = Vec::with_capacity(group_count);
        for group_index in 0..transition_group_count {
            let channel = self.executable_event_channel(layer_index, group_index);
            if group_index >= group_count {
                self.stop_current_event(
                    CurrentEventRoute::Primary,
                    CurrentEventStopRequest {
                        channel,
                        fade_seconds: transition_seconds,
                    },
                )?;
                continue;
            }
            let primary_group = &sequence.executable_event_tracks()[group_index];
            let interval_index = primary_group
                .intervals()
                .iter()
                .position(|interval| interval.sequence_end().get() > initial_seconds);

            let primary_slot = if let Some(interval_index) = interval_index {
                let interval = &primary_group.intervals()[interval_index];
                self.start_or_stop_initial_event(
                    CurrentEventRoute::Primary,
                    channel,
                    interval,
                    transition_seconds,
                    initial_seconds,
                )?;
                Some(self.new_current_event_slot(
                    layer_index,
                    group_index,
                    interval_index,
                    interval,
                )?)
            } else {
                self.stop_current_event(
                    CurrentEventRoute::Primary,
                    CurrentEventStopRequest {
                        channel,
                        fade_seconds: transition_seconds,
                    },
                )?;
                None
            };

            let payload_slot = match sequence.executable_payload_event_tracks().get(group_index) {
                Some(payload_group) => self.materialize_payload_slot(
                    layer_index,
                    group_index,
                    payload_group,
                    interval_index,
                    transition_seconds,
                    initial_seconds,
                )?,
                None => None,
            };
            primary.push(primary_slot);
            payload.push(payload_slot);
        }
        self.layers[layer_index].current_primary_event_tracks = primary;
        self.layers[layer_index].current_payload_event_tracks = payload;
        Ok(())
    }

    /// Materializes the payload slot aligned with one primary group.
    fn materialize_payload_slot(
        &mut self,
        layer_index: usize,
        group_index: usize,
        payload_group: &crate::PayloadEventTrackDefinition<E>,
        interval_index: Option<usize>,
        transition_seconds: f32,
        initial_seconds: f32,
    ) -> Result<CurrentEventSlot<E>, RuntimeError<M::Error, F::Error>> {
        // Native leaves the aligned payload slot alone while its owning module
        // cannot be resolved.
        if !self.payload_current_event_host_available(payload_group.owner()) {
            return Ok(None);
        }
        let channel = self.executable_event_channel(layer_index, group_index);
        let route = CurrentEventRoute::Payload(payload_group.owner());
        if let (Some(payload_track), Some(interval_index)) =
            (payload_group.executable_track(), interval_index)
        {
            let interval = &payload_track.intervals()[interval_index];
            self.start_or_stop_initial_event(
                route,
                channel,
                interval,
                transition_seconds,
                initial_seconds,
            )?;
            Ok(Some(self.new_current_event_slot(
                layer_index,
                group_index,
                interval_index,
                interval,
            )?))
        } else {
            self.stop_current_event(
                route,
                CurrentEventStopRequest {
                    channel,
                    fade_seconds: transition_seconds,
                },
            )?;
            Ok(None)
        }
    }

    /// Advances the fixed primary slots and their aligned payload slots in place.
    pub(crate) fn advance_current_event_tracks(
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
            .expect("validated current sequence must exist");
        let current_sequence_time = self.layers[layer_index].current_time_seconds;
        let previous_sequence_time = self.layers[layer_index].previous_time_seconds;
        let wrapped = self.layers[layer_index].wrapped;
        let has_wrapped = self.layers[layer_index].wrap_count != 0;
        let mut primary =
            std::mem::take(&mut self.layers[layer_index].current_primary_event_tracks);
        let mut payload =
            std::mem::take(&mut self.layers[layer_index].current_payload_event_tracks);

        let result = (|| {
            for group_index in 0..primary.len() {
                let channel = self.executable_event_channel(layer_index, group_index);
                let primary_group = &sequence.executable_event_tracks()[group_index];
                let mut primary_slot = match primary[group_index].take() {
                    Some(slot) => slot,
                    None if wrapped => {
                        let Some(interval) = primary_group.intervals().first() else {
                            continue;
                        };
                        self.start_update_event(CurrentEventRoute::Primary, channel, interval)?;
                        self.new_current_event_slot(layer_index, group_index, 0, interval)?
                    }
                    None => continue,
                };

                let target_index = target_interval_index(
                    primary_group,
                    primary_slot.interval_index,
                    current_sequence_time,
                    wrapped,
                    sequence.is_looping(),
                );
                let target_changed = target_index != primary_slot.interval_index;
                let interval = &primary_group.intervals()[target_index];
                if target_changed {
                    self.start_update_event(CurrentEventRoute::Primary, channel, interval)?;
                    primary_slot = self.rebind_current_event_slot(
                        layer_index,
                        primary_slot,
                        target_index,
                        interval,
                    )?;
                } else {
                    let boundary = interval.properties().restart_boundary_seconds();
                    if boundary > 0.0 && !has_wrapped {
                        if previous_sequence_time < boundary && boundary <= current_sequence_time {
                            self.start_update_event(CurrentEventRoute::Primary, channel, interval)?;
                            primary_slot.current_playback_seconds =
                                interval.playback_offset_seconds();
                        } else if current_sequence_time < boundary
                            && !self.current_event_gate(interval.event_root().event_id())?
                        {
                            primary[group_index] = Some(primary_slot);
                            continue;
                        }
                    }
                }

                let (candidate, normalized) =
                    current_playback(sequence.is_looping(), current_sequence_time, interval);
                primary_slot.previous_playback_seconds = primary_slot.current_playback_seconds;
                primary_slot.current_playback_seconds = candidate;
                self.update_current_event(
                    CurrentEventRoute::Primary,
                    CurrentEventUpdateRequest {
                        channel,
                        normalized_playback: normalized,
                        delta_seconds: delta_seconds * interval.effective_playback_scale(),
                    },
                )?;

                if let Some(payload_group) =
                    sequence.executable_payload_event_tracks().get(group_index)
                {
                    self.advance_payload_slot(
                        layer_index,
                        &mut payload[group_index],
                        &primary_slot,
                        payload_group,
                        PayloadAdvance {
                            group_index,
                            target_index,
                            target_changed,
                            normalized_playback: normalized,
                            delta_seconds: delta_seconds * interval.effective_playback_scale(),
                        },
                    )?;
                }
                primary[group_index] = Some(primary_slot);
            }
            Ok(())
        })();
        self.layers[layer_index].current_primary_event_tracks = primary;
        self.layers[layer_index].current_payload_event_tracks = payload;
        result
    }

    /// Advances the payload slot aligned with one already-advanced primary slot.
    fn advance_payload_slot(
        &mut self,
        layer_index: usize,
        slot: &mut CurrentEventSlot<E>,
        primary_slot: &CurrentEventTrackRuntime<E>,
        payload_group: &crate::PayloadEventTrackDefinition<E>,
        advance: PayloadAdvance,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        // Native leaves an existing payload slot byte-for-byte untouched while
        // its owning module cannot be resolved.
        if !self.payload_current_event_host_available(payload_group.owner()) {
            return Ok(());
        }
        let Some(payload_track) = payload_group.executable_track() else {
            return Ok(());
        };
        let payload_interval = &payload_track.intervals()[advance.target_index];
        let channel = self.executable_event_channel(layer_index, advance.group_index);
        let route = CurrentEventRoute::Payload(payload_group.owner());
        let (mut payload_slot, newly_materialized) = if let Some(existing) = slot.take() {
            (existing, false)
        } else {
            self.start_update_event(route, channel, payload_interval)?;
            (
                self.new_current_event_slot(
                    layer_index,
                    advance.group_index,
                    advance.target_index,
                    payload_interval,
                )?,
                true,
            )
        };
        if !newly_materialized
            && (advance.target_changed || payload_slot.interval_index != advance.target_index)
        {
            self.start_update_event(route, channel, payload_interval)?;
            payload_slot = self.rebind_current_event_slot(
                layer_index,
                payload_slot,
                advance.target_index,
                payload_interval,
            )?;
        }
        payload_slot.previous_playback_seconds = primary_slot.previous_playback_seconds;
        payload_slot.current_playback_seconds = primary_slot.current_playback_seconds;
        self.update_current_event(
            route,
            CurrentEventUpdateRequest {
                channel,
                normalized_playback: advance.normalized_playback,
                delta_seconds: advance.delta_seconds,
            },
        )?;
        *slot = Some(payload_slot);
        Ok(())
    }

    fn executable_event_channel(
        &self,
        layer_index: usize,
        group_index: usize,
    ) -> ExecutableEventChannel {
        let group = i32::try_from(group_index)
            .expect("validated executable group index must fit signed channel space");
        ExecutableEventChannel::new(
            self.layers[layer_index]
                .executable_event_channel_base
                .checked_add(group)
                .expect("validated executable channel base must not overflow"),
        )
    }

    fn new_current_event_slot(
        &mut self,
        layer_index: usize,
        group_index: usize,
        interval_index: usize,
        interval: &EventIntervalDefinition<E>,
    ) -> Result<CurrentEventTrackRuntime<E>, RuntimeError<M::Error, F::Error>> {
        let runtime_id = self.allocate_event_runtime_id()?;
        let callbacks = self.build_current_event_callbacks(layer_index, interval)?;
        Ok(CurrentEventTrackRuntime {
            group_index,
            interval_index,
            previous_playback_seconds: interval.playback_offset_seconds(),
            current_playback_seconds: interval.playback_offset_seconds(),
            runtime_id,
            callbacks: callbacks.into_boxed_slice(),
        })
    }

    fn rebind_current_event_slot(
        &mut self,
        layer_index: usize,
        mut slot: CurrentEventTrackRuntime<E>,
        interval_index: usize,
        interval: &EventIntervalDefinition<E>,
    ) -> Result<CurrentEventTrackRuntime<E>, RuntimeError<M::Error, F::Error>> {
        slot.interval_index = interval_index;
        slot.previous_playback_seconds = interval.playback_offset_seconds();
        slot.current_playback_seconds = interval.playback_offset_seconds();
        let callbacks = self.build_current_event_callbacks(layer_index, interval)?;
        slot.callbacks = callbacks.into_boxed_slice();
        Ok(slot)
    }

    fn build_current_event_callbacks(
        &mut self,
        layer_index: usize,
        interval: &EventIntervalDefinition<E>,
    ) -> Result<CurrentEventCallbackSlots<E>, RuntimeError<M::Error, F::Error>> {
        let nesting_base = self.layer_driver[layer_index].callback_nesting_base;
        let mut runtime_ids = std::collections::BTreeSet::new();
        let mut callbacks = Vec::with_capacity(interval.callbacks().len());
        for definition in interval.callbacks() {
            let callback = self.initialize_interval_callback_instance(
                layer_index,
                definition,
                crate::IntervalCallbackScope::CurrentEvent,
                crate::runtime::CallbackRegistrationTarget::Layer { layer_index },
            )?;
            let runtime_id = crate::CallbackRuntimeId::new(
                nesting_base.wrapping_add(definition.authored_id().get()),
            );
            if runtime_ids.insert(runtime_id) {
                callbacks.push(Some(CurrentEventCallbackRuntime {
                    instance_id: self.allocate_callback_instance_id()?,
                    start_seconds: definition.start().get(),
                    end_seconds: definition.end().get(),
                    state: CurrentEventCallbackState {
                        runtime_id,
                        active: false,
                        stopped: false,
                        deferred_exit: false,
                        may_defer: definition.may_defer(),
                    },
                    payload: Some(callback),
                }));
            }
        }
        Ok(callbacks)
    }

    fn start_or_stop_initial_event(
        &mut self,
        route: CurrentEventRoute,
        channel: ExecutableEventChannel,
        interval: &EventIntervalDefinition<E>,
        transition_seconds: f32,
        initial_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if interval.properties().restart_boundary_seconds() > 0.0 {
            self.stop_current_event(
                route,
                CurrentEventStopRequest {
                    channel,
                    fade_seconds: 0.0,
                },
            )
        } else {
            let fade_seconds = if transition_seconds >= 0.0 {
                transition_seconds
            } else {
                interval.fade_duration_seconds()
            };
            self.start_current_event(
                route,
                CurrentEventStartRequest {
                    channel,
                    event_id: interval.event_root().event_id(),
                    fixed_weight: 1.0,
                    normalized_start: (initial_seconds + interval.playback_offset_seconds())
                        / interval.event_duration().get(),
                    fade_seconds,
                    authored_weight: interval.authored_weight(),
                    looping: interval.loops_playback(),
                },
            )
        }
    }

    fn start_update_event(
        &mut self,
        route: CurrentEventRoute,
        channel: ExecutableEventChannel,
        interval: &EventIntervalDefinition<E>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.start_current_event(
            route,
            CurrentEventStartRequest {
                channel,
                event_id: interval.event_root().event_id(),
                fixed_weight: 1.0,
                normalized_start: interval.playback_offset_seconds()
                    / interval.event_duration().get(),
                fade_seconds: interval.fade_duration_seconds(),
                authored_weight: interval.authored_weight(),
                looping: interval.loops_playback(),
            },
        )
    }
}

/// Picks the interval a primary slot advances to for this update.
///
/// A wrap always restarts at the first interval. Otherwise the slot only moves
/// once sequence time passes its interval's end and that interval does not loop
/// its own event root, and then only to the next authored interval or back to
/// the first when the sequence itself loops.
fn target_interval_index<E>(
    group: &crate::EventTrackDefinition<E>,
    current_index: usize,
    sequence_time: f32,
    wrapped: bool,
    sequence_loops: bool,
) -> usize {
    if wrapped {
        return 0;
    }
    let current = &group.intervals()[current_index];
    if sequence_time < current.sequence_end().get() || current.loops_playback() {
        return current_index;
    }
    if current_index + 1 < group.intervals().len() {
        current_index + 1
    } else if sequence_loops {
        0
    } else {
        current_index
    }
}

fn current_playback<E>(
    sequence_loops: bool,
    sequence_time: f32,
    interval: &EventIntervalDefinition<E>,
) -> (f32, f32) {
    let duration = interval.event_duration().get();
    let scale = interval.effective_playback_scale();
    let mut candidate = ((sequence_time - interval.sequence_start().get())
        + interval.playback_offset_seconds())
        * scale;
    let mut normalized = candidate / duration;
    if normalized > 1.0 {
        if interval.loops_playback() || sequence_loops {
            candidate %= duration;
            normalized = candidate / duration;
        } else {
            normalized = 1.0;
        }
    }
    (candidate, normalized.clamp(0.0, 1.0))
}
