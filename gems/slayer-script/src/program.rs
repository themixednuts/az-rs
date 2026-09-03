//! Immutable compiled `SlayerScript` program tables.

use thiserror::Error;

use crate::{
    EventTrackDefinition, LayerId, ModuleId, ParentSequenceContext, SequenceId, StateTable,
    StateUpdateSelectorId,
};

mod validation;

pub use validation::ProgramValidationError;

/// A finite, non-negative number of seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct DurationSeconds(f32);

impl DurationSeconds {
    /// Zero seconds.
    pub const ZERO: Self = Self(0.0);

    /// Validates a duration used by runtime time operations.
    ///
    /// # Errors
    ///
    /// Returns [`DurationSecondsError`] for negative or non-finite input.
    pub const fn new(seconds: f32) -> Result<Self, DurationSecondsError> {
        if !seconds.is_finite() {
            return Err(DurationSecondsError::NotFinite);
        }
        if seconds < 0.0 {
            return Err(DurationSecondsError::Negative);
        }
        Ok(Self(seconds))
    }

    /// Returns this duration as seconds.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Why a duration cannot enter a compiled program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DurationSecondsError {
    /// NaN and infinity do not define deterministic time progression.
    #[error("duration must be finite")]
    NotFinite,
    /// Runtime duration only advances forward.
    #[error("duration must not be negative")]
    Negative,
}

/// A finite authored frame value converted at native 30 Hz.
///
/// Negative transition frames select native authored/default fade behavior;
/// negative initial-time frames remain a valid pre-roll time.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct AuthoredFrames(f32);

impl AuthoredFrames {
    /// Zero authored frames.
    pub const ZERO: Self = Self(0.0);
    /// Native `SlayerScript` authored frame rate.
    pub const FRAMES_PER_SECOND: f32 = 30.0;

    /// Creates a finite authored frame value.
    ///
    /// # Errors
    ///
    /// Returns [`AuthoredFramesError`] for NaN or infinity.
    pub const fn new(frames: f32) -> Result<Self, AuthoredFramesError> {
        if frames.is_finite() {
            Ok(Self(frames))
        } else {
            Err(AuthoredFramesError)
        }
    }

    /// Returns the authored frame value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Converts authored frames using the native `1 / 30` scale.
    #[must_use]
    pub const fn seconds(self) -> f32 {
        self.0 / Self::FRAMES_PER_SECOND
    }
}

/// An authored frame value was NaN or infinite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("authored frame value must be finite")]
pub struct AuthoredFramesError;

/// A finite multiplier applied to update delta for one layer.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct LayerPlaybackRate(f32);

impl LayerPlaybackRate {
    /// Native default layer playback rate.
    pub const UNITY: Self = Self(1.0);

    /// Creates a finite layer playback rate.
    ///
    /// Zero and negative rates are valid. Zero leaves normal-layer time
    /// unchanged; negative values drive the recovered current-layer clock
    /// backward.
    ///
    /// # Errors
    ///
    /// Returns [`LayerPlaybackRateError`] for NaN or infinity.
    pub const fn new(rate: f32) -> Result<Self, LayerPlaybackRateError> {
        if rate.is_finite() {
            Ok(Self(rate))
        } else {
            Err(LayerPlaybackRateError)
        }
    }

    /// Returns the scalar applied to caller delta.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for LayerPlaybackRate {
    fn default() -> Self {
        Self::UNITY
    }
}

/// A layer playback rate was NaN or infinite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("layer playback rate must be finite")]
pub struct LayerPlaybackRateError;

/// One immutable compiled sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceDefinition<O, E = ()> {
    duration: DurationSeconds,
    looping: bool,
    wrap_non_looping_at_end: bool,
    requires_normal_transition: bool,
    parent_sequence: Option<SequenceId>,
    parent_context: ParentSequenceContext,
    actions: Box<[O]>,
    authored_primary_event_group_count: AuthoredEventGroupCount,
    embedded_primary_event_tracks: Box<[EventTrackDefinition<E>]>,
    embedded_payload_event_tracks: Box<[EventTrackDefinition<E>]>,
    executable_primary_event_tracks: Box<[EventTrackDefinition<E>]>,
    executable_payload_event_tracks: Box<[PayloadEventTrackDefinition<E>]>,
}

/// One authored payload group and its optional compiled executable slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PayloadEventTrackDefinition<E> {
    owner: ModuleId,
    executable_track: Option<EventTrackDefinition<E>>,
}

impl<E> PayloadEventTrackDefinition<E> {
    /// Creates a payload group with a compiled executable slot.
    #[must_use]
    pub const fn new(owner: ModuleId, track: EventTrackDefinition<E>) -> Self {
        Self {
            owner,
            executable_track: Some(track),
        }
    }

    /// Creates an authored payload group whose executable table has no slot.
    #[must_use]
    pub const fn without_executable_slot(owner: ModuleId) -> Self {
        Self {
            owner,
            executable_track: None,
        }
    }

    #[must_use]
    pub const fn owner(&self) -> ModuleId {
        self.owner
    }

    /// Returns the compiled payload slot, when this sequence produced one.
    #[must_use]
    pub const fn executable_track(&self) -> Option<&EventTrackDefinition<E>> {
        self.executable_track.as_ref()
    }
}

/// Explicit authored primary-group count used by transition cleanup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct AuthoredEventGroupCount(u32);

impl AuthoredEventGroupCount {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl<O, E> SequenceDefinition<O, E> {
    /// Creates a sequence with no parent, operations, or event tracks.
    #[must_use]
    pub fn new(duration: DurationSeconds, looping: bool) -> Self {
        Self {
            duration,
            looping,
            wrap_non_looping_at_end: false,
            requires_normal_transition: false,
            parent_sequence: None,
            parent_context: ParentSequenceContext::default(),
            actions: Box::default(),
            authored_primary_event_group_count: AuthoredEventGroupCount::ZERO,
            embedded_primary_event_tracks: Box::default(),
            embedded_payload_event_tracks: Box::default(),
            executable_primary_event_tracks: Box::default(),
            executable_payload_event_tracks: Box::default(),
        }
    }

    /// Preserves native flag bit 1 behavior for a non-looping sequence.
    ///
    /// When set, reaching the sequence duration wraps instead of reporting end.
    #[must_use]
    pub const fn with_wrap_non_looping_at_end(mut self, wrap: bool) -> Self {
        self.wrap_non_looping_at_end = wrap;
        self
    }

    /// Marks a sequence whose native entry disallows force-path semantics.
    ///
    /// A force request involving this sequence is downgraded to the normal
    /// transition path; it is not rejected.
    #[must_use]
    pub const fn with_requires_normal_transition(mut self, required: bool) -> Self {
        self.requires_normal_transition = required;
        self
    }

    /// Assigns the compiler-resolved immediate sequence parent.
    #[must_use]
    pub const fn with_parent_sequence(mut self, parent: SequenceId) -> Self {
        self.parent_sequence = Some(parent);
        self
    }

    /// Assigns the ordered typed actions behind this sequence dispatcher.
    ///
    /// Every action receives the complete native mask. Parent sequence
    /// dispatchers execute deepest-first before this one; splitting actions
    /// into phase-specific vectors would lose combined masks such as `0x8018`.
    #[must_use]
    pub fn with_actions(mut self, actions: impl Into<Box<[O]>>) -> Self {
        self.actions = actions.into();
        self
    }

    /// Assigns the compiler-resolved opaque `ParentSequenceChanged` context.
    #[must_use]
    pub const fn with_parent_context(mut self, context: ParentSequenceContext) -> Self {
        self.parent_context = context;
        self
    }

    /// Assigns the authored primary-group count used by transition cleanup.
    #[must_use]
    pub const fn with_authored_primary_event_group_count(
        mut self,
        count: AuthoredEventGroupCount,
    ) -> Self {
        self.authored_primary_event_group_count = count;
        self
    }

    /// Assigns event tracks embedded in each `0x150` transition record.
    #[must_use]
    pub fn with_embedded_event_tracks(
        mut self,
        event_tracks: impl Into<Box<[EventTrackDefinition<E>]>>,
    ) -> Self {
        self.embedded_primary_event_tracks = event_tracks.into();
        self
    }

    /// Assigns payload tracks embedded in each `0x150` transition record.
    #[must_use]
    pub fn with_embedded_payload_event_tracks(
        mut self,
        event_tracks: impl Into<Box<[EventTrackDefinition<E>]>>,
    ) -> Self {
        self.embedded_payload_event_tracks = event_tracks.into();
        self
    }

    /// Assigns fixed current-layer executable primary slots.
    #[must_use]
    pub fn with_executable_event_tracks(
        mut self,
        event_tracks: impl Into<Box<[EventTrackDefinition<E>]>>,
    ) -> Self {
        self.executable_primary_event_tracks = event_tracks.into();
        self
    }

    /// Assigns authored payload owners and their optional executable slots.
    #[must_use]
    pub fn with_executable_payload_event_tracks(
        mut self,
        event_tracks: impl Into<Box<[PayloadEventTrackDefinition<E>]>>,
    ) -> Self {
        self.executable_payload_event_tracks = event_tracks.into();
        self
    }

    /// Returns the sequence duration.
    #[must_use]
    pub const fn duration(&self) -> DurationSeconds {
        self.duration
    }

    /// Returns whether elapsed sequence time wraps at the duration.
    #[must_use]
    pub const fn is_looping(&self) -> bool {
        self.looping
    }

    /// Returns native flag bit 1's proven end behavior.
    #[must_use]
    pub const fn wraps_non_looping_at_end(&self) -> bool {
        self.wrap_non_looping_at_end
    }

    /// Returns whether this sequence downgrades force-path semantics.
    #[must_use]
    pub const fn requires_normal_transition(&self) -> bool {
        self.requires_normal_transition
    }

    /// Returns the ordered typed actions for this sequence dispatcher.
    #[must_use]
    pub const fn actions(&self) -> &[O] {
        &self.actions
    }

    /// Returns the immediate parent whose actions execute first.
    #[must_use]
    pub const fn parent_sequence(&self) -> Option<SequenceId> {
        self.parent_sequence
    }

    #[must_use]
    pub const fn parent_context(&self) -> ParentSequenceContext {
        self.parent_context
    }

    /// Returns the authored primary-group count used by transition cleanup.
    #[must_use]
    pub const fn authored_primary_event_group_count(&self) -> AuthoredEventGroupCount {
        self.authored_primary_event_group_count
    }

    /// Returns fixed executable primary slots in compiled order.
    #[must_use]
    pub const fn executable_event_tracks(&self) -> &[EventTrackDefinition<E>] {
        &self.executable_primary_event_tracks
    }

    /// Returns authored payload owners and optional compiled slots.
    #[must_use]
    pub const fn executable_payload_event_tracks(&self) -> &[PayloadEventTrackDefinition<E>] {
        &self.executable_payload_event_tracks
    }

    pub(crate) fn event_track(
        &self,
        payload: bool,
        group_index: usize,
    ) -> Option<&EventTrackDefinition<E>> {
        if payload {
            self.embedded_payload_event_tracks.get(group_index)
        } else {
            self.embedded_primary_event_tracks.get(group_index)
        }
    }

    pub(crate) fn event_track_count(&self, payload: bool) -> usize {
        if payload {
            self.embedded_payload_event_tracks.len()
        } else {
            self.embedded_primary_event_tracks.len()
        }
    }

    pub(crate) fn default_incoming_transition_seconds(&self) -> f32 {
        self.executable_primary_event_tracks
            .first()
            .and_then(|track| track.intervals().first())
            .or_else(|| {
                self.executable_payload_event_tracks
                    .first()
                    .and_then(PayloadEventTrackDefinition::executable_track)
                    .and_then(|track| track.intervals().first())
            })
            .map_or(0.0, |interval| interval.fade_duration_seconds().max(0.0))
    }
}

/// One immutable compiled layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerDefinition {
    kind: LayerKind,
    playback_rate: LayerPlaybackRate,
    executable_event_channel_count: ExecutableEventChannelCount,
    executable_event_channel_base: i32,
    state_update_selector: Option<StateUpdateSelectorId>,
    allowed_sequences: Box<[SequenceId]>,
}

/// Explicit signed channel count stored on one compiled native layer entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct ExecutableEventChannelCount(i32);

impl ExecutableEventChannelCount {
    pub const ZERO: Self = Self(0);

    /// Creates a nonnegative compiled channel count.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutableEventChannelCountError`] when `value` is negative;
    /// native channel bases accumulate as nonnegative signed words.
    pub const fn new(value: i32) -> Result<Self, ExecutableEventChannelCountError> {
        if value < 0 {
            Err(ExecutableEventChannelCountError)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// A compiled layer channel count was negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("executable event channel count must be nonnegative")]
pub struct ExecutableEventChannelCountError;

/// Native update domain for a compiled layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayerKind {
    /// Ordinary sequence layer updated before the auxiliary layer.
    #[default]
    Normal,
    /// Auxiliary layer finalized before normal transition weights advance.
    Auxiliary,
}

impl LayerDefinition {
    /// Creates one empty normal layer.
    ///
    /// Native instances do not preselect a sequence. The first layer-state
    /// dispatch or another explicit host input must request the first transition.
    #[must_use]
    pub fn new() -> Self {
        Self {
            kind: LayerKind::Normal,
            playback_rate: LayerPlaybackRate::UNITY,
            executable_event_channel_count: ExecutableEventChannelCount::ZERO,
            executable_event_channel_base: 0,
            state_update_selector: None,
            allowed_sequences: Box::default(),
        }
    }

    /// Marks this as the native auxiliary update/finalization layer.
    #[must_use]
    pub const fn with_kind(mut self, kind: LayerKind) -> Self {
        self.kind = kind;
        self
    }

    /// Assigns the finite scalar applied to update delta for this layer.
    #[must_use]
    pub const fn with_playback_rate(mut self, playback_rate: LayerPlaybackRate) -> Self {
        self.playback_rate = playback_rate;
        self
    }

    /// Assigns the compiler-provided native channel count for this layer.
    #[must_use]
    pub const fn with_executable_event_channel_count(
        mut self,
        count: ExecutableEventChannelCount,
    ) -> Self {
        self.executable_event_channel_count = count;
        self
    }

    /// Assigns the compiler-provided sequence table owned by this layer.
    #[must_use]
    pub fn with_sequences(mut self, sequences: impl Into<Box<[SequenceId]>>) -> Self {
        self.allowed_sequences = sequences.into();
        self
    }

    /// Assigns the compiler-resolved optional per-layer state selector.
    #[must_use]
    pub const fn with_state_update_selector(mut self, selector: StateUpdateSelectorId) -> Self {
        self.state_update_selector = Some(selector);
        self
    }

    /// Returns this layer's native update domain.
    #[must_use]
    pub const fn kind(&self) -> LayerKind {
        self.kind
    }

    /// Returns the scalar applied to caller delta for this layer.
    #[must_use]
    pub const fn playback_rate(&self) -> LayerPlaybackRate {
        self.playback_rate
    }

    /// Returns the compiler-provided channel count.
    #[must_use]
    pub const fn executable_event_channel_count(&self) -> ExecutableEventChannelCount {
        self.executable_event_channel_count
    }

    pub(crate) const fn executable_event_channel_base(&self) -> i32 {
        self.executable_event_channel_base
    }

    /// Returns the dense sequences the compiler bound to this layer.
    #[must_use]
    pub const fn sequences(&self) -> &[SequenceId] {
        &self.allowed_sequences
    }

    #[must_use]
    pub const fn state_update_selector(&self) -> Option<StateUpdateSelectorId> {
        self.state_update_selector
    }

    pub(crate) fn allows_sequence(&self, sequence: SequenceId) -> bool {
        self.allowed_sequences.contains(&sequence)
    }
}

impl Default for LayerDefinition {
    fn default() -> Self {
        Self::new()
    }
}

/// A validated, shareable compiled `SlayerScript` program.
#[derive(Debug, Clone, PartialEq)]
pub struct SlayerProgram<O, E = ()> {
    sequences: Box<[SequenceDefinition<O, E>]>,
    layers: Box<[LayerDefinition]>,
    states: StateTable<O>,
}

impl<O, E> SlayerProgram<O, E> {
    /// Returns one compiled sequence.
    #[must_use]
    pub fn sequence(&self, id: SequenceId) -> Option<&SequenceDefinition<O, E>> {
        self.sequences.get(id.index())
    }

    /// Returns one compiled layer.
    #[must_use]
    pub fn layer(&self, id: LayerId) -> Option<&LayerDefinition> {
        self.layers.get(id.index())
    }

    /// Returns all compiled sequence definitions in dense identifier order.
    #[must_use]
    pub const fn sequences(&self) -> &[SequenceDefinition<O, E>] {
        &self.sequences
    }

    /// Returns all compiled layer definitions in dense identifier order.
    #[must_use]
    pub const fn layers(&self) -> &[LayerDefinition] {
        &self.layers
    }

    /// Returns the finalized compiler-ordered state table.
    #[must_use]
    pub const fn states(&self) -> &StateTable<O> {
        &self.states
    }
}
