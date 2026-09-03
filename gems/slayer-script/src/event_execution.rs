//! Embedded transition-record event playback (`0x150 + 0x48/+0x68`).
//!
//! This is intentionally not the fixed current-layer executable
//! `SequenceLayer +0x78/+0x98` lane, which lives in `current_event`.

use std::sync::Arc;

use crate::{
    EventTrackRuntime, ExternalPlaybackRequest, FunctionAdapter, ModuleAdapter, RuntimeError,
    RuntimeExecutor, SequenceActionMask, SequenceId, SequenceRuntimeId,
    runtime::CallbackRegistrationTarget,
};

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    pub(crate) fn update_layer(
        &mut self,
        layer_index: usize,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer_id = self.layers[layer_index].id;
        self.layers[layer_index].transition_count = 0;
        self.update_current_state(layer_index)?;
        let step = self.current_event_step(layer_index, delta_seconds)?;
        let program = Arc::clone(&self.program);
        if let Err(error) = self.layers[layer_index].advance_current_time(&program, step) {
            return Err(self.map_advance_error(layer_id, error));
        }
        if same_f32(
            self.layers[layer_index].previous_time_seconds,
            self.layers[layer_index].current_time_seconds,
        ) {
            return Ok(());
        }
        self.advance_current_event_tracks(layer_index, step)?;

        let mut restarts = 0_u8;
        loop {
            let mutation = self.layer_driver[layer_index].mutation_counter;
            let Some(sequence) = self.layers[layer_index].current() else {
                break;
            };
            let mut action_mask = SequenceActionMask::UPDATE;
            if self.layers[layer_index].wrapped || self.layers[layer_index].reached_end {
                action_mask = action_mask.with(SequenceActionMask::WRAP_OR_END);
            }
            self.execute_time_actions(
                layer_id,
                sequence,
                action_mask,
                CallbackRegistrationTarget::Layer { layer_index },
            )?;

            let (previous_seconds, current_seconds, force_exit) = {
                let layer = &self.layers[layer_index];
                (
                    layer.previous_time_seconds,
                    layer.current_time_seconds,
                    layer.wrapped || layer.reached_end,
                )
            };
            self.dispatch_retained_callbacks(
                layer_index,
                previous_seconds,
                current_seconds,
                force_exit,
                step,
                Some(layer_id),
            )?;
            self.apply_pending_state(layer_index)?;
            self.apply_pending_transition(layer_index, restarts)?;
            let restart = self.layer_driver[layer_index].mutation_counter != mutation;
            self.dispatch_current_event_callbacks(layer_index, step)?;
            if !restart {
                break;
            }
            restarts = restarts.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn update_auxiliary_layer(
        &mut self,
        layer_index: usize,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if delta_seconds == 0.0 {
            return Ok(());
        }
        let layer_id = self.layers[layer_index].id;
        let runtime_ids = self.layers[layer_index]
            .records
            .iter()
            .map(|record| record.runtime_id)
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            if self.record_index(layer_index, runtime_id).is_none() {
                continue;
            }
            self.advance_transition_record(layer_index, runtime_id, delta_seconds)?;
            let Some(index) = self.record_index(layer_index, runtime_id) else {
                continue;
            };
            if self.layers[layer_index].records[index].reached_end {
                self.layers[layer_index].records[index].exiting = true;
            }
            let record = &self.layers[layer_index].records[index];
            let sequence = record.sequence;
            let previous_time_seconds = record.previous_time_seconds;
            let current_time_seconds = record.current_time_seconds;
            let cumulative_time_seconds = record.cumulative_time_seconds;
            let wrap_count = record.wrap_count;
            let wrapped = record.wrapped;
            let reached_end = record.reached_end;
            let exiting = record.exiting;
            {
                let layer = &mut self.layers[layer_index];
                layer.projected_auxiliary_sequence = Some(sequence);
                layer.projected_auxiliary_runtime_id = Some(runtime_id);
                layer.previous_time_seconds = previous_time_seconds;
                layer.current_time_seconds = current_time_seconds;
                layer.cumulative_time_seconds = cumulative_time_seconds;
                layer.wrap_count = wrap_count;
                layer.wrapped = wrapped;
                layer.reached_end = reached_end;
            }
            let time_changed = !same_f32(previous_time_seconds, current_time_seconds);
            let mut action_mask = if previous_time_seconds < 0.0 {
                SequenceActionMask::INITIAL_UPDATE
            } else {
                SequenceActionMask::UPDATE
            };
            if wrapped || reached_end {
                action_mask = action_mask.with(SequenceActionMask::WRAP_OR_END);
            }
            if exiting {
                action_mask = action_mask.with(SequenceActionMask::EXIT);
            }
            if time_changed {
                self.execute_time_actions(
                    layer_id,
                    sequence,
                    action_mask,
                    CallbackRegistrationTarget::AuxiliaryRecord {
                        layer_index,
                        runtime_id,
                    },
                )?;
                self.dispatch_auxiliary_callbacks(
                    layer_index,
                    runtime_id,
                    previous_time_seconds,
                    current_time_seconds,
                    wrapped,
                    delta_seconds,
                )?;
            }
            if exiting && let Some(index) = self.record_index(layer_index, runtime_id) {
                self.stop_auxiliary_callbacks(layer_index, runtime_id)?;
                self.layers[layer_index].records[index].remove = true;
            }
        }
        self.compact_layer_records(layer_index);
        Ok(())
    }

    pub(crate) fn advance_transition_records(
        &mut self,
        layer_index: usize,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if delta_seconds <= 0.0 {
            return Ok(());
        }
        let runtime_ids = self.layers[layer_index]
            .records
            .iter()
            .filter(|record| !record.exiting)
            .map(|record| record.runtime_id)
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            self.advance_transition_record(layer_index, runtime_id, delta_seconds)?;
        }
        Ok(())
    }

    fn advance_transition_record(
        &mut self,
        layer_index: usize,
        runtime_id: SequenceRuntimeId,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let Some(index) = self.record_index(layer_index, runtime_id) else {
            return Ok(());
        };
        let layer_id = self.layers[layer_index].id;
        let program = Arc::clone(&self.program);
        if let Err(error) =
            self.layers[layer_index].records[index].advance_time(&program, delta_seconds)
        {
            return Err(self.map_advance_error(layer_id, error));
        }

        let (sequence, current_time, initialized) = {
            let record = &self.layers[layer_index].records[index];
            (
                record.sequence,
                record.current_time_seconds,
                record.embedded_event_tracks_initialized,
            )
        };
        if !initialized {
            let primary =
                self.build_initial_track_vector(layer_index, sequence, false, current_time)?;
            let payload =
                self.build_initial_track_vector(layer_index, sequence, true, current_time)?;
            let Some(index) = self.record_index(layer_index, runtime_id) else {
                return Ok(());
            };
            let record = &mut self.layers[layer_index].records[index];
            record.embedded_primary_event_tracks = primary;
            record.embedded_payload_event_tracks = payload;
            record.embedded_event_tracks_initialized = true;
        }

        let Some(index) = self.record_index(layer_index, runtime_id) else {
            return Ok(());
        };
        let (sequence, route_key, current_time, wrapped, mut primary, mut payload) = {
            let record = &mut self.layers[layer_index].records[index];
            (
                record.sequence,
                record.external_drive_route_key,
                record.current_time_seconds,
                record.wrapped,
                std::mem::take(&mut record.embedded_primary_event_tracks),
                std::mem::take(&mut record.embedded_payload_event_tracks),
            )
        };
        let result = self
            .advance_track_vector(
                layer_index,
                sequence,
                false,
                &mut primary,
                route_key,
                current_time,
                wrapped,
                delta_seconds,
            )
            .and_then(|()| {
                self.advance_track_vector(
                    layer_index,
                    sequence,
                    true,
                    &mut payload,
                    route_key,
                    current_time,
                    wrapped,
                    delta_seconds,
                )
            });
        primary.retain(|runtime| !runtime.remove);
        payload.retain(|runtime| !runtime.remove);
        if let Some(index) = self.record_index(layer_index, runtime_id) {
            let record = &mut self.layers[layer_index].records[index];
            record.embedded_primary_event_tracks = primary;
            record.embedded_payload_event_tracks = payload;
        }
        result
    }

    fn build_initial_track_vector(
        &mut self,
        layer_index: usize,
        sequence: SequenceId,
        payload_track: bool,
        current_time: f32,
    ) -> Result<Vec<EventTrackRuntime>, RuntimeError<M::Error, F::Error>> {
        let program = Arc::clone(&self.program);
        let definition = program
            .sequence(sequence)
            .expect("validated sequence must exist");
        let candidates = (0..definition.event_track_count(payload_track))
            .filter_map(|group_index| {
                let group = definition
                    .event_track(payload_track, group_index)
                    .expect("validated event group must exist");
                group
                    .intervals()
                    .iter()
                    .position(|interval| interval.sequence_end().get() > current_time)
                    .map(|interval_index| (group_index, interval_index))
            })
            .collect::<Vec<_>>();
        let mut records = Vec::with_capacity(candidates.len());
        for (group_index, interval_index) in candidates {
            records.push(self.instantiate_event_record(
                layer_index,
                sequence,
                payload_track,
                group_index,
                interval_index,
                current_time,
            )?);
        }
        Ok(records)
    }

    fn instantiate_event_record(
        &mut self,
        _layer_index: usize,
        sequence: SequenceId,
        payload_track: bool,
        group_index: usize,
        interval_index: usize,
        current_sequence_time: f32,
    ) -> Result<EventTrackRuntime, RuntimeError<M::Error, F::Error>> {
        let program = Arc::clone(&self.program);
        let sequence_definition = program
            .sequence(sequence)
            .expect("validated event sequence must exist");
        let interval = &sequence_definition
            .event_track(payload_track, group_index)
            .expect("validated event group must exist")
            .intervals()[interval_index];
        let event_runtime_id = self.allocate_event_runtime_id()?;
        let active = interval.sequence_start().get() <= current_sequence_time
            && current_sequence_time <= interval.sequence_end().get();
        let fade_duration = interval.fade_duration_seconds();
        Ok(EventTrackRuntime {
            group_index,
            interval_index,
            active,
            fading: false,
            remove: false,
            fade_duration_seconds: fade_duration,
            fade_elapsed_seconds: 0.0,
            previous_playback_seconds: 0.0,
            current_playback_seconds: 0.0,
            effective_weight: if active && fade_duration <= 0.0 {
                interval.authored_weight()
            } else {
                0.0
            },
            runtime_id: event_runtime_id,
            first_playback_update: true,
        })
    }

    // This is the cohesive embedded 0x150 event-record walker. Keeping
    // activation, successor replacement, playback, and fade updates together
    // preserves the source loop's same-visit insertion semantics. It must not
    // be reused for current-layer fixed slots.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn advance_track_vector(
        &mut self,
        layer_index: usize,
        sequence: SequenceId,
        payload_track: bool,
        tracks: &mut Vec<EventTrackRuntime>,
        route_key: crate::ExternalDriveRouteKey,
        current_sequence_time: f32,
        wrapped: bool,
        delta_seconds: f32,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let mut index = 0;
        // Native resets one sample slot per vector walk. The first external
        // record supplies the normalized fraction for every later external
        // record in this call, regardless of authored group.
        let mut external_normalized = None;
        while index < tracks.len() {
            let program = Arc::clone(&self.program);
            let sequence_definition = program
                .sequence(sequence)
                .expect("validated track sequence must exist");
            let group_index = tracks[index].group_index;
            let group = sequence_definition
                .event_track(payload_track, group_index)
                .expect("validated event group must exist");
            let current_interval_index = tracks[index].interval_index;
            let current_interval = &group.intervals()[current_interval_index];

            if !tracks[index].active
                && current_interval.sequence_start().get() <= current_sequence_time
                && current_sequence_time <= current_interval.sequence_end().get()
            {
                tracks[index].active = true;
            }

            if tracks[index].active
                && !tracks[index].fading
                && !wrapped
                && current_sequence_time >= current_interval.sequence_end().get()
            {
                if let Some(successor) = group.intervals().get(current_interval_index + 1) {
                    tracks[index].active = false;
                    tracks[index].fading = true;
                    tracks[index].fade_elapsed_seconds = 0.0;
                    tracks[index].fade_duration_seconds = successor.fade_duration_seconds();
                    let next = self.instantiate_event_record(
                        layer_index,
                        sequence,
                        payload_track,
                        group_index,
                        current_interval_index + 1,
                        current_sequence_time,
                    )?;
                    tracks.push(next);
                }
            } else if (wrapped || current_sequence_time < current_interval.sequence_start().get())
                && let Some(target_index) = group
                    .intervals()
                    .iter()
                    .rposition(|interval| interval.sequence_start().get() <= current_sequence_time)
                && target_index != current_interval_index
            {
                tracks[index].active = false;
                tracks[index].fading = true;
                tracks[index].fade_elapsed_seconds = 0.0;
                let target = &group.intervals()[target_index];
                tracks[index].fade_duration_seconds = target.fade_duration_seconds();
                tracks.push(self.instantiate_event_record(
                    layer_index,
                    sequence,
                    payload_track,
                    group_index,
                    target_index,
                    current_sequence_time,
                )?);
            }

            let interval_index = tracks[index].interval_index;
            let interval = &group.intervals()[interval_index];
            let step = PlaybackStep {
                route_key,
                sequence_time: current_sequence_time,
                delta_seconds,
                shared_external_normalized: external_normalized,
            };
            if let Some(normalized) =
                self.update_playback(layer_index, &mut tracks[index], interval, step)?
                && external_normalized.is_none()
            {
                external_normalized = Some(normalized);
            }
            update_event_weight(
                &mut tracks[index],
                interval.authored_weight(),
                delta_seconds,
            );
            index += 1;
        }
        Ok(())
    }

    fn update_playback(
        &mut self,
        layer_index: usize,
        runtime: &mut EventTrackRuntime,
        interval: &crate::EventIntervalDefinition<E>,
        step: PlaybackStep,
    ) -> Result<Option<f32>, RuntimeError<M::Error, F::Error>> {
        let PlaybackStep {
            route_key,
            sequence_time,
            delta_seconds,
            shared_external_normalized,
        } = step;
        runtime.previous_playback_seconds = runtime.current_playback_seconds;
        let duration = interval.event_duration().get();
        let scale = interval.effective_playback_scale();
        let (current, normalized) = if interval.is_externally_driven() {
            if let Some(normalized) = shared_external_normalized {
                (normalized * duration, Some(normalized))
            } else {
                let mut candidate = if interval.loops_playback()
                    || interval.playback_offset_seconds() <= runtime.current_playback_seconds
                {
                    let increment =
                        match self
                            .functions
                            .external_playback_increment(ExternalPlaybackRequest {
                                route_key,
                                runtime_id: runtime.runtime_id,
                                delta_seconds,
                            }) {
                            Ok(Some(increment)) => increment,
                            Ok(None) => {
                                self.poisoned = true;
                                return Err(RuntimeError::MissingExternalDriveAdapter {
                                    runtime_id: runtime.runtime_id,
                                });
                            }
                            Err(error) => return self.function_failure(error),
                        };
                    if !increment.is_finite() {
                        self.poisoned = true;
                        return Err(RuntimeError::InvalidExternalPlaybackIncrement {
                            runtime_id: runtime.runtime_id,
                        });
                    }
                    increment.mul_add(scale, runtime.current_playback_seconds)
                } else {
                    interval.playback_offset_seconds()
                };
                if interval.loops_playback() && candidate >= duration {
                    candidate %= duration;
                }
                (candidate, Some(candidate / duration))
            }
        } else {
            let mut candidate = ((sequence_time - interval.sequence_start().get())
                + interval.playback_offset_seconds())
                * scale;
            if interval.loops_playback() && candidate >= duration {
                candidate %= duration;
            }
            (candidate, None)
        };
        if !current.is_finite() {
            self.poisoned = true;
            return Err(RuntimeError::TimeOverflow {
                layer: self.layers[layer_index].id,
            });
        }
        runtime.current_playback_seconds = current.clamp(0.0, duration);
        if runtime.first_playback_update
            && is_zero(runtime.previous_playback_seconds)
            && runtime.current_playback_seconds > delta_seconds
        {
            runtime.previous_playback_seconds =
                (runtime.current_playback_seconds - delta_seconds).max(0.0);
        }
        runtime.first_playback_update = false;
        Ok(normalized)
    }

    fn record_index(&self, layer_index: usize, runtime_id: SequenceRuntimeId) -> Option<usize> {
        self.layers[layer_index]
            .records
            .iter()
            .position(|record| record.runtime_id == runtime_id)
    }

    pub(crate) fn compact_layer_records(&mut self, layer_index: usize) {
        self.layers[layer_index]
            .records
            .retain(|record| !record.remove);
        let live_runtime_ids = self.layers[layer_index]
            .records
            .iter()
            .map(|record| record.runtime_id)
            .collect::<std::collections::BTreeSet<_>>();
        self.layer_driver[layer_index]
            .auxiliary_callback_roots
            .retain(|runtime_id, _| live_runtime_ids.contains(runtime_id));
    }

    pub(crate) fn remove_zero_duration_exiting_records(&mut self, layer_index: usize) {
        self.layers[layer_index]
            .records
            .retain(|record| !(record.exiting && is_zero(record.transition_duration_seconds)));
        let live_runtime_ids = self.layers[layer_index]
            .records
            .iter()
            .map(|record| record.runtime_id)
            .collect::<std::collections::BTreeSet<_>>();
        self.layer_driver[layer_index]
            .auxiliary_callback_roots
            .retain(|runtime_id, _| live_runtime_ids.contains(runtime_id));
    }
}

/// The per-update inputs shared by every event record in one vector walk.
#[derive(Debug, Clone, Copy)]
struct PlaybackStep {
    /// Opaque native routing state handed to external-drive requests.
    route_key: crate::ExternalDriveRouteKey,
    /// Layer sequence time this update lands on.
    sequence_time: f32,
    /// Layer-scaled update delta.
    delta_seconds: f32,
    /// Normalized fraction an earlier external record already sampled.
    shared_external_normalized: Option<f32>,
}

fn update_event_weight(runtime: &mut EventTrackRuntime, authored_weight: f32, delta_seconds: f32) {
    runtime.fade_elapsed_seconds = if runtime.fade_duration_seconds <= 0.0 {
        0.0
    } else {
        (runtime.fade_elapsed_seconds + delta_seconds).clamp(0.0, runtime.fade_duration_seconds)
    };
    let ratio = if runtime.fade_duration_seconds <= 0.0 {
        1.0
    } else {
        (runtime.fade_elapsed_seconds / runtime.fade_duration_seconds).min(1.0)
    };
    runtime.effective_weight = if runtime.fading {
        (1.0 - ratio) * authored_weight
    } else {
        ratio * authored_weight
    };
    if runtime.fading && runtime.effective_weight <= 0.0 {
        runtime.remove = true;
    }
}

fn same_f32(left: f32, right: f32) -> bool {
    left.partial_cmp(&right) == Some(std::cmp::Ordering::Equal)
}

/// True for `+0.0` and `-0.0`: every bit below the sign bit is clear.
const fn is_zero(value: f32) -> bool {
    value.to_bits().trailing_zeros() >= 31
}
