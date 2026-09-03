//! Per-instance sequence-transition records and native interpolation.

use crate::{
    AuthoredFrames, CurrentEventTrackRuntime, EventTrackRuntime, ExternalDriveRouteKey,
    LayerDefinition, LayerId, LayerKind, SequenceId, SequenceRuntimeId, SlayerProgram,
};

/// Native `SlayerScript` accepts ten transition calls per layer between updates.
pub const MAX_TRANSITION_NESTING: u8 = 10;
/// Native default outgoing fade for a negative authored transition value.
pub const DEFAULT_OUTGOING_TRANSITION_SECONDS: f32 = 0.2;
/// Native infinite-duration sentinel that enables same-sequence record reuse.
pub const INFINITE_SEQUENCE_DURATION_SECONDS: f32 = f32::MAX;

/// Semantic replacement for native old/new sequence action masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencePhase {
    /// Operations selected by the old-sequence exit mask (`4`).
    Exit,
    /// Operations selected by the new-sequence enter mask (`0x8002`).
    Enter,
    /// Operations selected during sequence-time advancement.
    Update,
}

/// Exact native action-mask value associated with an operation invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceActionMask(u32);

impl SequenceActionMask {
    /// Old-sequence exit mask.
    pub const EXIT: Self = Self(4);
    /// New-sequence enter mask.
    pub const ENTER: Self = Self(0x8002);
    /// Ordinary changed-time mask.
    pub const UPDATE: Self = Self(0x8008);
    /// First changed-time mask when previous time is negative.
    pub const INITIAL_UPDATE: Self = Self(0x800a);
    /// Bit added on wrap or end.
    pub const WRAP_OR_END: Self = Self(0x10);

    /// Returns the exact native bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    pub(crate) const fn with(self, additional: Self) -> Self {
        Self(self.0 | additional.0)
    }
}

/// Proven sequence-change payload emitted after every applied transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceChanged {
    /// Layer whose current sequence changed.
    pub layer: LayerId,
    /// Sequence active before the transition.
    pub previous: Option<SequenceId>,
    /// Requested sequence, or `None` for clear.
    pub current: Option<SequenceId>,
}

/// Opaque signed parent/state identifier carried by `ParentSequenceChanged`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct ResolvedParentId(i32);

impl ResolvedParentId {
    pub const NONE: Self = Self(-1);

    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// Compiler-resolved parent payload retained without semantic reinterpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentSequenceContext {
    /// Resolved parent/state identifier.
    pub parent: ResolvedParentId,
    /// Exact 24-byte resolved state/literal value beginning at event `+0x28`.
    pub resolved_value_words: [u32; 6],
    /// Exact five-word layer/sequence state snapshot at event `+0x48..+0x58`.
    pub state_words: [u32; 5],
}

impl Default for ParentSequenceContext {
    fn default() -> Self {
        Self {
            parent: ResolvedParentId::NONE,
            resolved_value_words: [0; 6],
            state_words: [0; 5],
        }
    }
}

/// Proven custom-fanout event CRC `0x379c35ff` emitted after applied Trans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParentSequenceChanged {
    pub parent: ResolvedParentId,
    pub resolved_value_words: [u32; 6],
    pub transition_frames: f32,
    pub initial_time_frames: f32,
    pub state_words: [u32; 5],
    pub layer: LayerId,
}

/// Observable result of native's void transition entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// The transition transaction ran and emitted this change payload.
    Applied(SequenceChanged),
    /// The opaque instance-lifecycle gate returned before touching layer state.
    BlockedByLifecycle,
    /// The non-null target guard returned its native blocking value.
    BlockedByTarget { sequence: SequenceId },
    /// The persistent per-update transition counter exceeded ten calls.
    IgnoredNestingLimit,
    /// A guarded normal non-null request replaced the layer's pending slot.
    Deferred,
}

/// Exact public inputs recovered for native `SequenceLayer::Trans`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionRequest {
    next: Option<SequenceId>,
    transition_frames: AuthoredFrames,
    initial_time_frames: AuthoredFrames,
    force: bool,
}

impl TransitionRequest {
    /// Creates a transition request in authored 30 Hz frames.
    #[must_use]
    pub const fn new(
        next: Option<SequenceId>,
        transition_frames: AuthoredFrames,
        initial_time_frames: AuthoredFrames,
        force: bool,
    ) -> Self {
        Self {
            next,
            transition_frames,
            initial_time_frames,
            force,
        }
    }

    /// Creates a zero-duration, zero-initial-time, normal transition.
    #[must_use]
    pub const fn immediate(next: Option<SequenceId>) -> Self {
        Self::new(next, AuthoredFrames::ZERO, AuthoredFrames::ZERO, false)
    }

    /// Returns the requested next sequence, or `None` for explicit clear.
    #[must_use]
    pub const fn next(self) -> Option<SequenceId> {
        self.next
    }

    /// Returns the single native transition-duration input.
    #[must_use]
    pub const fn transition_frames(self) -> AuthoredFrames {
        self.transition_frames
    }

    /// Returns the incoming sequence's initial-time input.
    #[must_use]
    pub const fn initial_time_frames(self) -> AuthoredFrames {
        self.initial_time_frames
    }

    /// Returns whether the caller requested the native force path.
    #[must_use]
    pub const fn is_forced(self) -> bool {
        self.force
    }
}

/// Read-only state for one native `0x150` sequence-transition record.
///
/// The native byte size is documentation only and is not a Rust layout target.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct SequenceTransitionRuntime {
    pub(crate) sequence: SequenceId,
    pub(crate) runtime_id: SequenceRuntimeId,
    pub(crate) external_drive_route_key: ExternalDriveRouteKey,
    pub(crate) previous_time_seconds: f32,
    pub(crate) current_time_seconds: f32,
    pub(crate) cumulative_time_seconds: f32,
    pub(crate) wrap_count: u32,
    pub(crate) wrapped: bool,
    pub(crate) reached_end: bool,
    pub(crate) exiting: bool,
    pub(crate) remove: bool,
    pub(crate) transition_duration_seconds: f32,
    pub(crate) transition_elapsed_seconds: f32,
    pub(crate) raw_transition_progress: f32,
    pub(crate) effective_weight: f32,
    pub(crate) embedded_event_tracks_initialized: bool,
    pub(crate) embedded_primary_event_tracks: Vec<EventTrackRuntime>,
    pub(crate) embedded_payload_event_tracks: Vec<EventTrackRuntime>,
}

impl SequenceTransitionRuntime {
    pub(crate) fn incoming(
        runtime_id: SequenceRuntimeId,
        sequence: SequenceId,
        initial_time_seconds: f32,
        transition_duration_seconds: f32,
    ) -> Self {
        let immediate = transition_duration_seconds <= 0.0;
        Self {
            sequence,
            runtime_id,
            external_drive_route_key: ExternalDriveRouteKey::from_native(0),
            previous_time_seconds: 0.0,
            current_time_seconds: initial_time_seconds,
            cumulative_time_seconds: 0.0,
            wrap_count: 0,
            wrapped: false,
            reached_end: false,
            exiting: false,
            remove: false,
            transition_duration_seconds,
            transition_elapsed_seconds: 0.0,
            raw_transition_progress: if immediate { 1.0 } else { 0.0 },
            effective_weight: if immediate { 1.0 } else { 0.0 },
            embedded_event_tracks_initialized: false,
            embedded_primary_event_tracks: Vec::new(),
            embedded_payload_event_tracks: Vec::new(),
        }
    }

    /// Returns the compiled sequence identifier.
    #[must_use]
    pub const fn sequence(&self) -> SequenceId {
        self.sequence
    }

    /// Returns the monotonic instance-local runtime identifier.
    #[must_use]
    pub const fn runtime_id(&self) -> SequenceRuntimeId {
        self.runtime_id
    }

    /// Returns the opaque, zero-initialized native external-drive route word.
    #[must_use]
    pub const fn external_drive_route_key(&self) -> ExternalDriveRouteKey {
        self.external_drive_route_key
    }

    /// Returns previous wrapped sequence time.
    #[must_use]
    pub const fn previous_time_seconds(&self) -> f32 {
        self.previous_time_seconds
    }

    /// Returns current wrapped sequence time.
    #[must_use]
    pub const fn current_time_seconds(&self) -> f32 {
        self.current_time_seconds
    }

    /// Returns cumulative unwrapped sequence time.
    #[must_use]
    pub const fn cumulative_time_seconds(&self) -> f32 {
        self.cumulative_time_seconds
    }

    /// Returns the native wrap counter.
    #[must_use]
    pub const fn wrap_count(&self) -> u32 {
        self.wrap_count
    }

    /// Returns whether this update exposed a wrapped segment.
    #[must_use]
    pub const fn wrapped_this_step(&self) -> bool {
        self.wrapped
    }

    /// Returns whether a stopping sequence reached its duration.
    #[must_use]
    pub const fn reached_end(&self) -> bool {
        self.reached_end
    }

    /// Returns whether this is an outgoing record.
    #[must_use]
    pub const fn is_exiting(&self) -> bool {
        self.exiting
    }

    /// Returns the transition duration in seconds.
    #[must_use]
    pub const fn transition_duration_seconds(&self) -> f32 {
        self.transition_duration_seconds
    }

    /// Returns raw transition interpolation progress.
    #[must_use]
    pub const fn raw_transition_progress(&self) -> f32 {
        self.raw_transition_progress
    }

    /// Returns normalized composited transition weight.
    #[must_use]
    pub const fn effective_weight(&self) -> f32 {
        self.effective_weight
    }

    /// Returns embedded primary playback records advanced for blend state.
    ///
    /// Native does not route these records through the current layer's
    /// post-pending callback dispatcher.
    #[must_use]
    pub fn embedded_primary_event_tracks(&self) -> &[EventTrackRuntime] {
        &self.embedded_primary_event_tracks
    }

    /// Returns embedded payload playback records advanced for blend state.
    #[must_use]
    pub fn embedded_payload_event_tracks(&self) -> &[EventTrackRuntime] {
        &self.embedded_payload_event_tracks
    }

    pub(crate) fn advance_time<O, E>(
        &mut self,
        program: &SlayerProgram<O, E>,
        delta_seconds: f32,
    ) -> Result<(), SequenceAdvanceError> {
        let sequence = program
            .sequence(self.sequence)
            .expect("validated sequence record must reference the program");
        let cumulative = self.cumulative_time_seconds + delta_seconds;
        let current = self.current_time_seconds + delta_seconds;
        if !cumulative.is_finite() || !current.is_finite() {
            return Err(SequenceAdvanceError::TimeOverflow);
        }

        self.wrapped = false;
        self.reached_end = false;
        self.cumulative_time_seconds = cumulative;
        self.previous_time_seconds = self.current_time_seconds;
        self.current_time_seconds = current;

        let duration = sequence.duration().get();
        if self.current_time_seconds >= duration {
            if !sequence.is_looping() && !sequence.wraps_non_looping_at_end() {
                self.reached_end = true;
            } else {
                self.current_time_seconds %= duration;
                self.wrapped = true;
                self.wrap_count = self.wrap_count.saturating_add(1);
            }
        }
        self.current_time_seconds = self.current_time_seconds.clamp(0.0, duration);
        Ok(())
    }
}

/// Public read-only view of one layer's per-instance transition state.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct SequenceLayer<E = ()> {
    pub(crate) id: LayerId,
    pub(crate) kind: LayerKind,
    pub(crate) playback_rate: crate::LayerPlaybackRate,
    pub(crate) records: Vec<SequenceTransitionRuntime>,
    pub(crate) previous_time_seconds: f32,
    pub(crate) current_time_seconds: f32,
    pub(crate) cumulative_time_seconds: f32,
    pub(crate) wrap_count: u32,
    pub(crate) wrapped: bool,
    pub(crate) reached_end: bool,
    pub(crate) executable_event_channel_base: i32,
    pub(crate) current_primary_event_tracks: Vec<Option<CurrentEventTrackRuntime<E>>>,
    pub(crate) current_payload_event_tracks: Vec<Option<CurrentEventTrackRuntime<E>>>,
    pub(crate) transition_count: u8,
    pub(crate) projected_auxiliary_sequence: Option<SequenceId>,
    pub(crate) projected_auxiliary_runtime_id: Option<SequenceRuntimeId>,
}

impl<E> SequenceLayer<E> {
    pub(crate) const fn new(id: LayerId, definition: &LayerDefinition) -> Self {
        Self {
            id,
            kind: definition.kind(),
            playback_rate: definition.playback_rate(),
            records: Vec::new(),
            previous_time_seconds: 0.0,
            current_time_seconds: 0.0,
            cumulative_time_seconds: 0.0,
            wrap_count: 0,
            wrapped: false,
            reached_end: false,
            executable_event_channel_base: definition.executable_event_channel_base(),
            current_primary_event_tracks: Vec::new(),
            current_payload_event_tracks: Vec::new(),
            transition_count: 0,
            projected_auxiliary_sequence: None,
            projected_auxiliary_runtime_id: None,
        }
    }

    /// Returns this layer's dense program identifier.
    #[must_use]
    pub const fn id(&self) -> LayerId {
        self.id
    }

    /// Returns whether this is a normal or auxiliary layer.
    #[must_use]
    pub const fn kind(&self) -> LayerKind {
        self.kind
    }

    /// Returns this layer's scalar applied to caller delta.
    #[must_use]
    pub const fn playback_rate(&self) -> crate::LayerPlaybackRate {
        self.playback_rate
    }

    /// Returns the newest current sequence.
    #[must_use]
    pub fn current(&self) -> Option<SequenceId> {
        self.projected_auxiliary_sequence.or_else(|| {
            self.records
                .last()
                .filter(|record| !record.exiting)
                .map(|record| record.sequence)
        })
    }

    pub(crate) fn current_runtime_id(&self) -> Option<SequenceRuntimeId> {
        self.projected_auxiliary_runtime_id.or_else(|| {
            self.records
                .iter()
                .rfind(|record| !record.exiting)
                .map(|record| record.runtime_id)
        })
    }

    /// Returns coexisting transition records with newest last.
    #[must_use]
    pub fn records(&self) -> &[SequenceTransitionRuntime] {
        &self.records
    }

    /// Returns the fixed primary executable slots in channel order.
    #[must_use]
    pub fn current_primary_event_tracks(&self) -> &[Option<CurrentEventTrackRuntime<E>>] {
        &self.current_primary_event_tracks
    }

    /// Returns the index-aligned payload executable slots.
    #[must_use]
    pub fn current_payload_event_tracks(&self) -> &[Option<CurrentEventTrackRuntime<E>>] {
        &self.current_payload_event_tracks
    }

    /// Returns layer-level previous time reset by transition entry.
    #[must_use]
    pub const fn previous_time_seconds(&self) -> f32 {
        self.previous_time_seconds
    }

    /// Returns layer-level current time reset by transition entry.
    #[must_use]
    pub const fn current_time_seconds(&self) -> f32 {
        self.current_time_seconds
    }

    /// Returns layer-level cumulative time reset by transition entry.
    #[must_use]
    pub const fn cumulative_time_seconds(&self) -> f32 {
        self.cumulative_time_seconds
    }

    /// Returns the native current-sequence wrap counter.
    #[must_use]
    pub const fn wrap_count(&self) -> u32 {
        self.wrap_count
    }

    /// Returns whether current-sequence time wrapped during this update.
    #[must_use]
    pub const fn wrapped_this_step(&self) -> bool {
        self.wrapped
    }

    /// Returns whether the current non-looping sequence reached its end.
    #[must_use]
    pub const fn reached_end(&self) -> bool {
        self.reached_end
    }

    pub(crate) fn reset_time(&mut self, initial_time_seconds: f32) {
        self.current_time_seconds = initial_time_seconds;
        self.cumulative_time_seconds = initial_time_seconds;
        self.previous_time_seconds = initial_time_seconds - 1.0 / AuthoredFrames::FRAMES_PER_SECOND;
        self.wrapped = false;
        self.reached_end = false;
    }

    pub(crate) fn advance_current_time<O, P>(
        &mut self,
        program: &SlayerProgram<O, P>,
        delta_seconds: f32,
    ) -> Result<(), SequenceAdvanceError> {
        self.wrapped = false;
        self.reached_end = false;
        let Some(sequence) = self.current() else {
            return Ok(());
        };
        let definition = program
            .sequence(sequence)
            .expect("validated current sequence must reference the program");
        let cumulative = self.cumulative_time_seconds + delta_seconds;
        let current = self.current_time_seconds + delta_seconds;
        if !cumulative.is_finite() || !current.is_finite() {
            return Err(SequenceAdvanceError::TimeOverflow);
        }
        self.previous_time_seconds = self.current_time_seconds;
        self.current_time_seconds = current;
        self.cumulative_time_seconds = cumulative;

        let duration = definition.duration().get();
        if self.current_time_seconds >= duration {
            if !definition.is_looping() && !definition.wraps_non_looping_at_end() {
                self.reached_end = true;
            } else {
                self.wrapped = true;
                self.wrap_count = self.wrap_count.saturating_add(1);
            }
        }
        self.current_time_seconds = self.current_time_seconds.clamp(0.0, duration);
        Ok(())
    }

    pub(crate) fn update_weights(&mut self, delta_seconds: f32) {
        if delta_seconds <= 0.0 || self.records.is_empty() {
            return;
        }

        let oldest_elapsed = self.records[0].transition_elapsed_seconds;
        for record in &mut self.records {
            if record.exiting {
                continue;
            }
            if record.transition_duration_seconds <= 0.0 {
                record.transition_elapsed_seconds = 0.0;
                record.raw_transition_progress = 1.0;
            } else {
                record.transition_elapsed_seconds = (record.transition_elapsed_seconds
                    + delta_seconds)
                    .clamp(0.0, record.transition_duration_seconds);
                record.raw_transition_progress =
                    record.transition_elapsed_seconds / record.transition_duration_seconds;
            }
        }

        if self.records.last().is_some_and(|record| record.exiting) {
            for record in &mut self.records {
                record.effective_weight = if record.transition_duration_seconds <= 0.0 {
                    0.0
                } else {
                    (record.effective_weight - delta_seconds / record.transition_duration_seconds)
                        .max(0.0)
                };
            }
        } else if self.records.len() > 1 {
            self.records[0].effective_weight = 1.0;
            let first = &mut self.records[0];
            if first.exiting
                && first.transition_duration_seconds > 0.0
                && first.transition_duration_seconds < DEFAULT_OUTGOING_TRANSITION_SECONDS
            {
                first.transition_elapsed_seconds = (first.transition_elapsed_seconds
                    + delta_seconds)
                    .clamp(0.0, first.transition_duration_seconds);
                first.effective_weight =
                    (1.0 - oldest_elapsed / first.transition_duration_seconds).max(0.0);
            }
            for newer_index in 1..self.records.len() {
                let newer_weight = self.records[newer_index].raw_transition_progress;
                self.records[newer_index].effective_weight = newer_weight;
                for preceding in &mut self.records[..newer_index] {
                    preceding.effective_weight *= 1.0 - newer_weight;
                }
            }
            smooth_and_normalize(&mut self.records);
        } else if !self.records[0].exiting {
            self.records[0].effective_weight = self.records[0].raw_transition_progress;
        }

        for record in &mut self.records {
            if record.effective_weight <= 0.0 {
                record.remove = true;
            }
        }
    }
}

fn smooth_and_normalize(records: &mut [SequenceTransitionRuntime]) {
    for record in &mut *records {
        let centered = record.effective_weight.clamp(0.0, 1.0) - 0.5;
        record.effective_weight = centered / (2.0 * centered).mul_add(centered, 0.5) + 0.5;
    }
    let total = records
        .iter()
        .map(|record| record.effective_weight)
        .sum::<f32>();
    if total > 0.0 {
        for record in records {
            record.effective_weight /= total;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SequenceAdvanceError {
    TimeOverflow,
}
