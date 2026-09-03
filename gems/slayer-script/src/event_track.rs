//! Typed event-track program data and callback phases.

use thiserror::Error;

use crate::{DurationSeconds, EventRuntimeId};

/// Opaque 32-bit identifier stored at the native executable event root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ExecutableEventId(u32);

impl ExecutableEventId {
    /// Preserves the raw compiled identifier without assigning domain meaning.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw compiled value passed to the typed host.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Event-root data referenced by one interval descriptor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventRootDefinition {
    event_id: ExecutableEventId,
    duration: DurationSeconds,
    externally_driven: bool,
}

impl EventRootDefinition {
    /// Creates one event-root reference.
    ///
    /// # Errors
    ///
    /// Externally driven roots require a positive duration because native
    /// derives a normalized playback fraction from that duration.
    pub fn new(
        event_id: ExecutableEventId,
        duration: DurationSeconds,
        externally_driven: bool,
    ) -> Result<Self, EventTrackValidationError> {
        if externally_driven && duration == DurationSeconds::ZERO {
            return Err(EventTrackValidationError::NonPositiveExternalDuration);
        }
        Ok(Self {
            event_id,
            duration,
            externally_driven,
        })
    }

    /// Returns the opaque executable event identifier.
    #[must_use]
    pub const fn event_id(self) -> ExecutableEventId {
        self.event_id
    }

    /// Returns the event-root duration.
    #[must_use]
    pub const fn duration(self) -> DurationSeconds {
        self.duration
    }

    /// Returns whether current playback comes from the typed host service.
    #[must_use]
    pub const fn is_externally_driven(self) -> bool {
        self.externally_driven
    }
}

/// Parallel bound-track properties for one event interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundEventProperties {
    playback_scale: f32,
    restart_boundary_seconds: f32,
    playback_offset_seconds: f32,
    fade_duration_seconds: f32,
    authored_weight: f32,
    loop_playback: bool,
}

impl BoundEventProperties {
    /// Creates finite properties mirroring the native `0x40` definition.
    ///
    /// # Errors
    ///
    /// Returns [`EventTrackValidationError::NonFiniteScalar`] naming the first
    /// of `playback_scale`, `restart_boundary_seconds`,
    /// `playback_offset_seconds`, `fade_duration_seconds`, or
    /// `authored_weight` that is NaN or infinite; native playback state cannot
    /// hold either.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        playback_scale: f32,
        restart_boundary_seconds: f32,
        playback_offset_seconds: f32,
        fade_duration_seconds: f32,
        authored_weight: f32,
        loop_playback: bool,
    ) -> Result<Self, EventTrackValidationError> {
        for (field, value) in [
            (EventTrackScalar::PlaybackScale, playback_scale),
            (EventTrackScalar::RestartBoundary, restart_boundary_seconds),
            (EventTrackScalar::PlaybackOffset, playback_offset_seconds),
            (EventTrackScalar::FadeDuration, fade_duration_seconds),
            (EventTrackScalar::AuthoredWeight, authored_weight),
        ] {
            if !value.is_finite() {
                return Err(EventTrackValidationError::NonFiniteScalar { field });
            }
        }
        Ok(Self {
            playback_scale,
            restart_boundary_seconds,
            playback_offset_seconds,
            fade_duration_seconds,
            authored_weight,
            loop_playback,
        })
    }

    /// Returns authored scale; zero is evaluated as one by native playback.
    #[must_use]
    pub const fn playback_scale(self) -> f32 {
        self.playback_scale
    }

    /// Returns the opaque positive restart/gate boundary at properties `+0x18`.
    #[must_use]
    pub const fn restart_boundary_seconds(self) -> f32 {
        self.restart_boundary_seconds
    }

    /// Returns authored playback offset.
    #[must_use]
    pub const fn playback_offset_seconds(self) -> f32 {
        self.playback_offset_seconds
    }

    /// Returns authored fade duration.
    #[must_use]
    pub const fn fade_duration_seconds(self) -> f32 {
        self.fade_duration_seconds
    }

    /// Returns authored event weight.
    #[must_use]
    pub const fn authored_weight(self) -> f32 {
        self.authored_weight
    }

    /// Returns whether event-root playback loops.
    #[must_use]
    pub const fn loops_playback(self) -> bool {
        self.loop_playback
    }
}

/// One ordered callback object bound to playback-time bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalCallbackDefinition<E> {
    authored_id: crate::CallbackAuthoredId,
    start: DurationSeconds,
    end: DurationSeconds,
    may_defer: bool,
    payload: E,
}

impl<E> IntervalCallbackDefinition<E> {
    /// Creates a callback interval with inclusive start and exclusive end.
    ///
    /// Native accepts the stored bounds verbatim; reversed bounds simply never
    /// satisfy the ordinary active-window predicate.
    pub const fn new(
        authored_id: crate::CallbackAuthoredId,
        start: DurationSeconds,
        end: DurationSeconds,
        payload: E,
    ) -> Self {
        Self {
            authored_id,
            start,
            end,
            may_defer: false,
            payload,
        }
    }

    /// Returns the callback-local compiler identifier.
    #[must_use]
    pub const fn authored_id(&self) -> crate::CallbackAuthoredId {
        self.authored_id
    }

    /// Selects native callback vslot `+0x58` behavior when deferral is allowed.
    ///
    /// Deferrable callbacks may append a queued phase record from retained-tree
    /// dispatch. Stop instead marks deferred EXIT and migrates the node; flush
    /// executes that EXIT directly before draining queued records.
    #[must_use]
    pub const fn with_deferred_dispatch(mut self, enabled: bool) -> Self {
        self.may_defer = enabled;
        self
    }

    /// Returns the callback's playback-time start.
    #[must_use]
    pub const fn start(&self) -> DurationSeconds {
        self.start
    }

    /// Returns the callback's playback-time end.
    #[must_use]
    pub const fn end(&self) -> DurationSeconds {
        self.end
    }

    /// Returns the project-defined typed callback payload.
    #[must_use]
    pub const fn payload(&self) -> &E {
        &self.payload
    }

    /// Returns whether this callback's native vslot permits retained deferral.
    #[must_use]
    pub const fn may_defer(&self) -> bool {
        self.may_defer
    }
}

/// One authored sequence-time interval and its bound event track.
///
/// Event-root duration and external-drive state are intentionally per interval,
/// matching the native descriptor's event/root pointer. Playback scale, offset,
/// fade, weight, and looping come from the parallel bound-track definition.
#[derive(Debug, Clone, PartialEq)]
pub struct EventIntervalDefinition<E> {
    sequence_start: DurationSeconds,
    sequence_end: DurationSeconds,
    event_root: EventRootDefinition,
    properties: BoundEventProperties,
    callbacks: Box<[IntervalCallbackDefinition<E>]>,
}

impl<E> EventIntervalDefinition<E> {
    /// Creates one validated authored interval.
    ///
    /// A zero authored scale retains native meaning and evaluates as `1.0` at
    /// runtime. Non-positive fade duration remains valid and means immediate
    /// full/zero fade weight.
    ///
    /// # Errors
    ///
    /// Returns [`EventTrackValidationError`] for reversed bounds or a
    /// non-positive looping event duration.
    pub fn new(
        sequence_start: DurationSeconds,
        sequence_end: DurationSeconds,
        event_root: EventRootDefinition,
        properties: BoundEventProperties,
        callbacks: impl Into<Box<[IntervalCallbackDefinition<E>]>>,
    ) -> Result<Self, EventTrackValidationError> {
        if sequence_end < sequence_start {
            return Err(EventTrackValidationError::IntervalEndBeforeStart);
        }
        if properties.loops_playback() && event_root.duration() == DurationSeconds::ZERO {
            return Err(EventTrackValidationError::NonPositiveLoopDuration);
        }
        Ok(Self {
            sequence_start,
            sequence_end,
            event_root,
            properties,
            callbacks: callbacks.into(),
        })
    }

    /// Returns the inclusive sequence-time start.
    #[must_use]
    pub const fn sequence_start(&self) -> DurationSeconds {
        self.sequence_start
    }

    /// Returns the exclusive sequence-time successor boundary.
    #[must_use]
    pub const fn sequence_end(&self) -> DurationSeconds {
        self.sequence_end
    }

    /// Returns this interval's event-root duration.
    #[must_use]
    pub const fn event_duration(&self) -> DurationSeconds {
        self.event_root.duration()
    }

    /// Returns the event-root data referenced by this descriptor.
    #[must_use]
    pub const fn event_root(&self) -> EventRootDefinition {
        self.event_root
    }

    /// Returns the parallel bound-track properties.
    #[must_use]
    pub const fn properties(&self) -> BoundEventProperties {
        self.properties
    }

    /// Returns whether playback comes from the injected external-drive host.
    #[must_use]
    pub const fn is_externally_driven(&self) -> bool {
        self.event_root.is_externally_driven()
    }

    /// Returns authored playback scale; zero has effective scale `1.0`.
    #[must_use]
    pub const fn playback_scale(&self) -> f32 {
        self.properties.playback_scale()
    }

    pub(crate) const fn effective_playback_scale(&self) -> f32 {
        if is_zero(self.properties.playback_scale()) {
            1.0
        } else {
            self.properties.playback_scale()
        }
    }

    /// Returns authored playback offset in seconds.
    #[must_use]
    pub const fn playback_offset_seconds(&self) -> f32 {
        self.properties.playback_offset_seconds()
    }

    /// Returns authored fade duration in seconds.
    #[must_use]
    pub const fn fade_duration_seconds(&self) -> f32 {
        self.properties.fade_duration_seconds()
    }

    /// Returns authored event weight.
    #[must_use]
    pub const fn authored_weight(&self) -> f32 {
        self.properties.authored_weight()
    }

    /// Returns whether playback wraps at the event-root duration.
    #[must_use]
    pub const fn loops_playback(&self) -> bool {
        self.properties.loops_playback()
    }

    /// Returns callback objects in native ordered-map order.
    #[must_use]
    pub const fn callbacks(&self) -> &[IntervalCallbackDefinition<E>] {
        &self.callbacks
    }
}

/// One native event-track group with index-aligned interval definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct EventTrackDefinition<E> {
    intervals: Box<[EventIntervalDefinition<E>]>,
}

impl<E> EventTrackDefinition<E> {
    /// Preserves authored order for native group-local `index + 1` lookup.
    ///
    /// # Errors
    ///
    /// Returns no error today: every per-interval invariant is already
    /// enforced by [`EventIntervalDefinition::new`], and authored order is
    /// taken verbatim. The `Result` is kept so a group-level rule can be added
    /// without breaking callers.
    pub fn new(
        intervals: impl Into<Box<[EventIntervalDefinition<E>]>>,
    ) -> Result<Self, EventTrackValidationError> {
        Ok(Self {
            intervals: intervals.into(),
        })
    }

    /// Returns index-aligned interval definitions in authored order.
    #[must_use]
    pub const fn intervals(&self) -> &[EventIntervalDefinition<E>] {
        &self.intervals
    }
}

/// One callback phase bit recovered from the native 12-byte deferred record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EventCallbackPhase {
    /// Callback object left its active interval.
    Exit = 4,
    /// Callback object entered its active interval.
    Enter = 2,
    /// Callback object remains active for this update.
    Update = 8,
}

/// Opaque native external-drive routing state (`SequenceTransitionRuntime + 0x18`).
///
/// Fresh native records initialize this word to zero and infinite self-reuse
/// preserves it. No setter is exposed until the native producer is proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ExternalDriveRouteKey(u32);

impl ExternalDriveRouteKey {
    pub(crate) const fn from_native(value: u32) -> Self {
        Self(value)
    }

    /// Returns the preserved native value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// External playback-increment request pinned from native vslot `+0x88`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExternalPlaybackRequest {
    /// Opaque sequence-record routing state from native offset `+0x18`.
    pub route_key: ExternalDriveRouteKey,
    /// Live event-track runtime identifier from native offset `+0x28`.
    pub runtime_id: EventRuntimeId,
    /// Exact layer-scaled update delta.
    pub delta_seconds: f32,
}

/// Scalar field rejected during event-track validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTrackScalar {
    /// Bound-track playback scale.
    PlaybackScale,
    /// Opaque current-lane restart/gate boundary.
    RestartBoundary,
    /// Bound-track authored offset.
    PlaybackOffset,
    /// Bound-track fade duration.
    FadeDuration,
    /// Bound-track authored weight.
    AuthoredWeight,
}

/// Why an authored event-track group cannot enter a compiled program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EventTrackValidationError {
    /// A sequence-time interval is reversed.
    #[error("event-track interval end precedes its start")]
    IntervalEndBeforeStart,
    /// A looping event root cannot use zero duration.
    #[error("looping event duration must be positive")]
    NonPositiveLoopDuration,
    /// Externally driven normalization requires positive event duration.
    #[error("externally driven event duration must be positive")]
    NonPositiveExternalDuration,
    /// Native floating-point state cannot accept NaN or infinity.
    #[error("event-track scalar {field:?} must be finite")]
    NonFiniteScalar { field: EventTrackScalar },
}

/// Read-only state for one live event-track interval record.
// The four flags mirror independent native state bytes on the event-track
// record and are set and read separately; no enum models them.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct EventTrackRuntime {
    pub(crate) group_index: usize,
    pub(crate) interval_index: usize,
    pub(crate) active: bool,
    pub(crate) fading: bool,
    pub(crate) remove: bool,
    pub(crate) fade_duration_seconds: f32,
    pub(crate) fade_elapsed_seconds: f32,
    pub(crate) previous_playback_seconds: f32,
    pub(crate) current_playback_seconds: f32,
    pub(crate) effective_weight: f32,
    pub(crate) runtime_id: EventRuntimeId,
    pub(crate) first_playback_update: bool,
}

// `active`, `stopped`, and `deferred_exit` are independent native per-callback
// state bytes that can all be set at once, and `may_defer` is the compiled
// vslot selection; no enum models the combination.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy)]
pub struct CurrentEventCallbackState {
    pub(crate) runtime_id: crate::CallbackRuntimeId,
    pub(crate) active: bool,
    pub(crate) stopped: bool,
    pub(crate) deferred_exit: bool,
    pub(crate) may_defer: bool,
}

#[derive(Debug, Clone)]
pub struct CurrentEventCallbackRuntime<E> {
    pub(crate) instance_id: u64,
    pub(crate) start_seconds: f32,
    pub(crate) end_seconds: f32,
    pub(crate) state: CurrentEventCallbackState,
    pub(crate) payload: Option<E>,
}

/// One fixed current-layer executable event slot.
#[derive(Debug, Clone)]
pub struct CurrentEventTrackRuntime<E = ()> {
    pub(crate) group_index: usize,
    pub(crate) interval_index: usize,
    pub(crate) previous_playback_seconds: f32,
    pub(crate) current_playback_seconds: f32,
    pub(crate) runtime_id: EventRuntimeId,
    pub(crate) callbacks: Box<[Option<CurrentEventCallbackRuntime<E>>]>,
}

impl<E> CurrentEventTrackRuntime<E> {
    #[must_use]
    pub const fn group_index(&self) -> usize {
        self.group_index
    }

    #[must_use]
    pub const fn interval_index(&self) -> usize {
        self.interval_index
    }

    #[must_use]
    pub const fn previous_playback_seconds(&self) -> f32 {
        self.previous_playback_seconds
    }

    #[must_use]
    pub const fn playback_seconds(&self) -> f32 {
        self.current_playback_seconds
    }

    #[must_use]
    pub const fn runtime_id(&self) -> EventRuntimeId {
        self.runtime_id
    }
}

impl EventTrackRuntime {
    /// Returns the native event runtime identifier.
    #[must_use]
    pub const fn runtime_id(&self) -> EventRuntimeId {
        self.runtime_id
    }

    /// Returns the group-local authored interval index.
    #[must_use]
    pub const fn interval_index(&self) -> usize {
        self.interval_index
    }

    /// Returns whether this is the current interval record.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Returns whether a successor superseded this record.
    #[must_use]
    pub const fn is_fading(&self) -> bool {
        self.fading
    }

    /// Returns current event-root playback time in seconds.
    #[must_use]
    pub const fn playback_seconds(&self) -> f32 {
        self.current_playback_seconds
    }

    /// Returns the authored weight after fade-in or fade-out.
    #[must_use]
    pub const fn effective_weight(&self) -> f32 {
        self.effective_weight
    }
}

/// True for `+0.0` and `-0.0`: every bit below the sign bit is clear.
const fn is_zero(value: f32) -> bool {
    value.to_bits().trailing_zeros() >= 31
}
