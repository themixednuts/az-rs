//! Typed boundaries between the generic executor and project behavior.

use std::error::Error;

use crate::{
    CallbackAuthoredId, DurationSeconds, EventCallbackPhase, ExecutableEventId,
    ExternalDriveRouteKey, ExternalPlaybackRequest, LayerId, ModuleId, OnStart,
    ParentSequenceChanged, RuntimeContext, SequenceActionMask, SequenceChanged, SequenceId,
    SequencePhase, StateChanged, StateId, StateOperationInvocation, UpdateLayerStates,
};

/// Native callback registration target selecting one of the two proved paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalCallbackScope {
    /// Sequence action registration into the active retained/auxiliary root.
    Sequence,
    /// Registration into the active fixed current-event slot.
    CurrentEvent,
}

/// Bind data supplied before callback initialization and runtime-ID insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalCallbackBinding {
    pub layer: LayerId,
    pub authored_id: CallbackAuthoredId,
    pub scope: IntervalCallbackScope,
}

/// One immutable sequence operation selected by a semantic phase.
#[derive(Debug, Clone, Copy)]
pub struct OperationInvocation<'a, O> {
    layer: LayerId,
    sequence: SequenceId,
    phase: SequencePhase,
    action_mask: SequenceActionMask,
    operation: &'a O,
}

impl<'a, O> OperationInvocation<'a, O> {
    pub(crate) const fn new(
        layer: LayerId,
        sequence: SequenceId,
        phase: SequencePhase,
        action_mask: SequenceActionMask,
        operation: &'a O,
    ) -> Self {
        Self {
            layer,
            sequence,
            phase,
            action_mask,
            operation,
        }
    }

    /// Returns the layer executing the operation.
    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    /// Returns the sequence executing the operation.
    #[must_use]
    pub const fn sequence(&self) -> SequenceId {
        self.sequence
    }

    /// Returns the semantic enter/exit selection.
    #[must_use]
    pub const fn phase(&self) -> SequencePhase {
        self.phase
    }

    /// Returns the exact native action-mask bits for this invocation.
    #[must_use]
    pub const fn action_mask(&self) -> SequenceActionMask {
        self.action_mask
    }

    /// Returns the immutable compiled operation.
    #[must_use]
    pub const fn operation(&self) -> &'a O {
        self.operation
    }
}

/// Non-null target presented to the native host transition guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionGuard {
    /// Layer receiving the transition.
    pub layer: LayerId,
    /// Current newest sequence, when present.
    pub current: Option<SequenceId>,
    /// Requested non-null target.
    pub next: SequenceId,
}

/// One native current-layer event channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ExecutableEventChannel(i32);

impl ExecutableEventChannel {
    pub(crate) const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the compiled native channel value.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// Exact typed request passed to native host slot `+0x60`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentEventStartRequest {
    pub channel: ExecutableEventChannel,
    pub event_id: ExecutableEventId,
    pub fixed_weight: f32,
    pub normalized_start: f32,
    pub fade_seconds: f32,
    pub authored_weight: f32,
    pub looping: bool,
}

/// Exact typed request passed to native host slot `+0x68`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentEventStopRequest {
    pub channel: ExecutableEventChannel,
    pub fade_seconds: f32,
}

/// Exact typed request passed to native host slot `+0x78`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentEventUpdateRequest {
    pub channel: ExecutableEventChannel,
    pub normalized_playback: f32,
    pub delta_seconds: f32,
}

/// Current-layer external-step request passed to native host slot `+0x90`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentEventStepRequest {
    pub route_key: ExternalDriveRouteKey,
    pub delta_seconds: f32,
}

/// One direct callback from a fixed current-layer slot.
#[derive(Debug)]
pub struct IntervalCallbackInvocation<'a, E> {
    callback_runtime_id: crate::CallbackRuntimeId,
    phase: EventCallbackPhase,
    delta_seconds: f32,
    payload: &'a mut E,
}

impl<'a, E> IntervalCallbackInvocation<'a, E> {
    pub(crate) const fn new(
        callback_runtime_id: crate::CallbackRuntimeId,
        phase: EventCallbackPhase,
        delta_seconds: f32,
        payload: &'a mut E,
    ) -> Self {
        Self {
            callback_runtime_id,
            phase,
            delta_seconds,
            payload,
        }
    }

    #[must_use]
    pub const fn callback_runtime_id(&self) -> crate::CallbackRuntimeId {
        self.callback_runtime_id
    }

    #[must_use]
    pub const fn phase(&self) -> EventCallbackPhase {
        self.phase
    }

    #[must_use]
    pub const fn delta_seconds(&self) -> f32 {
        self.delta_seconds
    }

    #[must_use]
    pub const fn payload(&self) -> &E {
        &*self.payload
    }

    #[must_use]
    pub const fn payload_mut(&mut self) -> &mut E {
        self.payload
    }
}

/// Typed host boundary for fixed current-layer event slots.
pub trait CurrentEventHost<E> {
    /// Host fault propagated through the owning function adapter.
    type Error: Error + Send + Sync + 'static;

    /// Begins playback on one fixed slot.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the host cannot start the slot: the channel
    /// names no installed fixed slot, or `request.event_id` resolves to no
    /// playable event root.
    fn start_current_event(&mut self, request: CurrentEventStartRequest)
    -> Result<(), Self::Error>;

    /// Fades one fixed slot out over `request.fade_seconds`.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the channel names no installed fixed slot or
    /// the host cannot begin the requested fade. Stopping an already-idle slot
    /// is a native-compatible no-op, not a fault.
    fn stop_current_event(&mut self, request: CurrentEventStopRequest) -> Result<(), Self::Error>;

    /// Advances one live fixed slot to a normalized playback position.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the channel names no live slot or the host
    /// rejects the normalized position it is handed.
    fn update_current_event(
        &mut self,
        request: CurrentEventUpdateRequest,
    ) -> Result<(), Self::Error>;

    /// Native `+0x80`; `false` skips this primary and aligned payload update.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the host cannot evaluate the gate for
    /// `event_id`. `Ok(false)` is the native-compatible skip, not a fault.
    fn current_event_gate(&mut self, event_id: ExecutableEventId) -> Result<bool, Self::Error>;

    /// Native `+0x90`; replaces the current-layer step before its clock moves.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the host cannot produce a step for
    /// `request.route_key`. A non-finite `Ok` value is rejected separately by
    /// the runtime as `RuntimeError::InvalidCurrentEventStep`.
    fn replace_current_event_step(
        &mut self,
        request: CurrentEventStepRequest,
    ) -> Result<f32, Self::Error>;
}

/// Project functions invoked by generic sequence and interval behavior.
pub trait FunctionAdapter<O, E, M>: Sized
where
    M: ModuleAdapter<O, E, Self>,
{
    /// Typed project-function failure propagated by the runtime.
    type Error: Error + Send + Sync + 'static;

    /// Returns the optional fixed-slot event host.
    ///
    /// Runtime validation permits executable tracks only when the instance
    /// installs this capability; absence becomes a typed runtime failure.
    fn current_event_host(&mut self) -> Option<&mut dyn CurrentEventHost<E, Error = Self::Error>> {
        None
    }

    /// Executes one typed immutable sequence operation.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when execution fails.
    fn execute_operation(
        &mut self,
        invocation: OperationInvocation<'_, O>,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Executes one callback object owned by the instance functions object.
    ///
    /// The module adapter is explicitly lent to the callback so a call to
    /// [`RuntimeContext::trans`] can reenter the runtime synchronously without
    /// interior mutability or unsafe aliasing.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the callback body fails. A
    /// callback that reenters the runtime through [`RuntimeContext::trans`]
    /// reports that failure through the context instead, not here.
    fn execute_interval_callback(
        &mut self,
        invocation: IntervalCallbackInvocation<'_, E>,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Binds one newly cloned callback object to its runtime instance/context.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the clone cannot be attached to
    /// `binding.layer` — for example when the callback's authored ID names no
    /// project-side object. A failure here aborts the registration and the
    /// callback is never initialized.
    fn bind_interval_callback(
        &mut self,
        binding: IntervalCallbackBinding,
        callback: &mut E,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Runs native callback initialization after both bind operations.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the callback's own
    /// initialization fails. The runtime treats that as fatal for the
    /// registration and does not install the object.
    fn initialize_interval_callback(
        &mut self,
        binding: IntervalCallbackBinding,
        callback: &mut E,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Finalizes one runtime-owned callback object before its node is erased.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when teardown fails. Like every other
    /// adapter fault it surfaces as `RuntimeError::Function` and poisons the
    /// runtime, because the erasure it accompanies is already partly applied.
    fn finalize_interval_callback(
        &mut self,
        callback: &mut E,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Executes one ordered action from a finalized state record.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the action body fails. The
    /// runtime restores the enclosing action mask and abandons the remaining
    /// actions of that ENTER/EXIT/UPDATE pass.
    fn execute_state_operation(
        &mut self,
        invocation: StateOperationInvocation<'_, O>,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Dispatches the optional compiled per-layer `UpdateLayerStates` selector.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the selector object cannot be
    /// evaluated. `Ok(false)` means the selector declined and ordinary state
    /// UPDATE actions run instead; that is not a fault.
    fn dispatch_update_layer_states(
        &mut self,
        event: UpdateLayerStates,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<bool, Self::Error>;

    /// Executes the selector object's unresolved alternate vslot when selected.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when that alternate body fails. It is
    /// reached only after [`Self::dispatch_update_layer_states`] answered
    /// `Ok(true)` for the same event.
    fn execute_selected_state_update(
        &mut self,
        event: UpdateLayerStates,
        modules: &mut M,
        context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error>;

    /// Native state-runtime vslot `+0x80`; `true` silently blocks a request.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the guard cannot be evaluated
    /// for `current` -> `next`. `Ok(true)` is the silent native-compatible
    /// block and leaves the layer's current state untouched.
    fn state_change_blocked(
        &mut self,
        layer: LayerId,
        current: StateId,
        next: StateId,
    ) -> Result<bool, Self::Error>;

    /// Refreshes state-derived layer metadata after installing a new state.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the refresh fails. It is called
    /// between the EXIT and ENTER action passes, so a failure here leaves
    /// `layer` holding the new state with stale derived metadata.
    fn refresh_state_layer_metadata(&mut self, layer: LayerId) -> Result<(), Self::Error>;

    /// Applies the opaque instance-lifecycle gate before a transition call.
    ///
    /// Native returns before incrementing the layer transition counter when
    /// this reports `true`. Implementations own the paired host-state update
    /// that occurs on the allowed path; the engine does not assign domain names
    /// to the two recovered bytes.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the instance-lifecycle state
    /// cannot be read. `Ok(true)` is the silent block, not a fault.
    fn transition_application_blocked(&mut self) -> Result<bool, Self::Error>;

    /// Applies the non-null native host transition guard.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when guard evaluation fails. Native
    /// treats `Ok(true)` as a silent blocked transition, not a runtime fault.
    fn blocks_transition_target(&mut self, guard: TransitionGuard) -> Result<bool, Self::Error>;

    /// Requests an externally-driven playback increment.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when the host service fails. `Ok(None)`
    /// reports that no external-drive adapter is installed and the runtime will
    /// hard-fail instead of substituting `dt`.
    fn external_playback_increment(
        &mut self,
        request: ExternalPlaybackRequest,
    ) -> Result<Option<f32>, Self::Error>;

    /// Receives the proven sequence-change payload.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when change handling fails.
    fn on_sequence_changed(
        &mut self,
        _event: SequenceChanged,
        _modules: &mut M,
        _context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Receives typed state-change CRC `0xd736574f` after state ENTER actions.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when change handling fails. The
    /// default implementation ignores the event and never fails.
    fn on_state_changed(
        &mut self,
        _event: StateChanged,
        _modules: &mut M,
        _context: &mut RuntimeContext<'_, O, E, M, Self>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Typed module set responsible for direct event routing.
pub trait ModuleAdapter<O, E, F>: Sized
where
    F: FunctionAdapter<O, E, Self>,
{
    /// Closed typed event family routed to one owning module.
    type TypedEvent;
    /// Closed custom event family accepted by one or more modules.
    type CustomEvent;
    /// Typed module-routing failure propagated by the runtime.
    type Error: Error + Send + Sync + 'static;

    /// Resolves the module-owned functions object for an aligned payload track.
    fn current_event_host(
        &mut self,
        _owner: ModuleId,
    ) -> Option<&mut dyn CurrentEventHost<E, Error = Self::Error>> {
        None
    }

    /// Dispatches native's first-update `OnStart` event (`0x8b372fca`).
    ///
    /// # Errors
    ///
    /// Unknown ownership is a native-compatible no-op. Errors are reserved for
    /// failures inside a module that elected to handle the event.
    fn dispatch_on_start(
        &mut self,
        event: OnStart,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;

    /// Synchronously dispatches a typed event to its owning module.
    ///
    /// # Errors
    ///
    /// Unknown ownership is a native-compatible no-op. Errors are reserved for
    /// failures inside a module that elected to handle the event.
    fn dispatch_typed(
        &mut self,
        event: &Self::TypedEvent,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;

    /// Synchronously dispatches a custom event to one named module.
    ///
    /// # Errors
    ///
    /// Unknown or rejecting targets are native-compatible no-ops. Errors are
    /// reserved for failures inside a module that accepted the event.
    fn dispatch_custom_targeted(
        &mut self,
        target: ModuleId,
        event: &Self::CustomEvent,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;

    /// Synchronously dispatches to all accepting modules in stable order.
    ///
    /// # Errors
    ///
    /// Zero recipients is a native-compatible no-op. Errors are reserved for
    /// failures inside an accepting module.
    fn dispatch_custom_fanout(
        &mut self,
        event: &Self::CustomEvent,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;

    /// Always fanouts generic `ParentSequenceChanged` CRC `0x379c35ff`.
    ///
    /// # Errors
    ///
    /// Every module receives this event, so errors are reserved for failures
    /// inside a module that acted on it. Modules that ignore it report
    /// `Ok(())`.
    fn dispatch_parent_sequence_changed(
        &mut self,
        event: ParentSequenceChanged,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;

    /// Updates registered module instances after sequence and weight updates.
    ///
    /// # Errors
    ///
    /// Returns the project-defined error when module update fails.
    fn update(
        &mut self,
        delta: DurationSeconds,
        functions: &mut F,
        context: &mut RuntimeContext<'_, O, E, Self, F>,
    ) -> Result<(), Self::Error>;
}
