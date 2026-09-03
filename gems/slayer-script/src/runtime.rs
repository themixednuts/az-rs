//! Deterministic per-instance `SlayerScript` execution.

use std::{
    collections::{BTreeMap, VecDeque},
    error::Error as StdError,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use thiserror::Error;

use crate::{
    CallbackRuntimeId, CurrentEventRoute, CurrentEventTrackRuntime, CustomDispatchTarget,
    DurationSeconds, EventRuntimeId, FunctionAdapter, LayerId, LayerKind, ModuleAdapter, OnStart,
    RuntimeContext, SequenceId, SequenceLayer, SequenceRuntimeId, SlayerProgram, TransitionOutcome,
    TransitionRequest, sequence::SequenceAdvanceError, state::LayerStateRuntime,
};

#[derive(Debug)]
pub struct LayerDriverState<E> {
    pub(crate) pending_transition: Option<PendingTransition>,
    pub(crate) mutation_counter: u64,
    pub(crate) callback_nesting_base: u32,
    pub(crate) retained_callbacks: BTreeMap<CallbackRuntimeId, RetainedCallbackObject<E>>,
    pub(crate) auxiliary_callback_roots:
        BTreeMap<SequenceRuntimeId, BTreeMap<CallbackRuntimeId, RetainedCallbackObject<E>>>,
    pub(crate) inflight_current_event_dispatch: Option<InflightCurrentEventDispatch<E>>,
    pub(crate) inflight_current_event_slot: Option<InflightCurrentEventSlot<E>>,
    pub(crate) inflight_callbacks: Vec<InflightCallbackObject<E>>,
    pub(crate) callback_queue: VecDeque<DeferredCallbackWork>,
    pub(crate) state: LayerStateRuntime,
}

impl<E> Default for LayerDriverState<E> {
    fn default() -> Self {
        Self {
            pending_transition: None,
            mutation_counter: 0,
            callback_nesting_base: 0,
            retained_callbacks: BTreeMap::new(),
            auxiliary_callback_roots: BTreeMap::new(),
            inflight_current_event_dispatch: None,
            inflight_current_event_slot: None,
            inflight_callbacks: Vec::new(),
            callback_queue: VecDeque::new(),
            state: LayerStateRuntime::default(),
        }
    }
}

#[derive(Debug)]
pub struct InflightCurrentEventDispatch<E> {
    pub(crate) sequence: SequenceId,
    pub(crate) primary: Vec<Option<CurrentEventTrackRuntime<E>>>,
    pub(crate) payload: Vec<Option<CurrentEventTrackRuntime<E>>>,
}

#[derive(Debug)]
pub struct InflightCurrentEventSlot<E> {
    pub(crate) route: CurrentEventRoute,
    pub(crate) slot: CurrentEventTrackRuntime<E>,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingTransition {
    pub(crate) request: TransitionRequest,
}

#[derive(Debug)]
pub struct RetainedCallbackObject<E> {
    pub(crate) callback: crate::CurrentEventCallbackRuntime<E>,
}

#[derive(Debug)]
pub struct InflightCallbackObject<E> {
    pub(crate) target: CallbackRegistrationTarget,
    pub(crate) object: RetainedCallbackObject<E>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackRegistrationTarget {
    Layer {
        layer_index: usize,
    },
    AuxiliaryRecord {
        layer_index: usize,
        runtime_id: SequenceRuntimeId,
    },
    CurrentSlot {
        layer_index: usize,
        event_runtime_id: EventRuntimeId,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct DeferredCallbackWork {
    pub(crate) callback_runtime_id: CallbackRuntimeId,
    pub(crate) phases: DeferredCallbackPhases,
    pub(crate) delta_seconds: f32,
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(transparent)]
pub struct DeferredCallbackPhases(u32);

impl DeferredCallbackPhases {
    pub(crate) const ENTER: Self = Self(0x2);
    pub(crate) const EXIT: Self = Self(0x4);
    pub(crate) const UPDATE: Self = Self(0x8);

    pub(crate) const fn contains(self, phase: Self) -> bool {
        self.0 & phase.0 != 0
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn insert(&mut self, phase: Self) {
        self.0 |= phase.0;
    }
}

/// Whether native current-event host slots `+0x60/+0x68/+0x78/+0x90` execute.
///
/// The recovered runtime stores this as an opaque instance byte. The semantic
/// source that selects the byte is not yet named, so construction requires an
/// explicit capability choice instead of guessing a client/server meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentEventHostExecution {
    /// Invoke current-event start, stop, update, and external-step host slots.
    Enabled,
    /// Maintain runtime slots and callbacks but suppress those four host calls.
    Suppressed,
}

pub struct RuntimeState<O, E> {
    pub(crate) program: Arc<SlayerProgram<O, E>>,
    pub(crate) layers: Box<[SequenceLayer<E>]>,
    pub(crate) layer_driver: Box<[LayerDriverState<E>]>,
    pub(crate) next_sequence_runtime_id: u32,
    pub(crate) next_event_runtime_id: u32,
    pub(crate) next_callback_instance_id: u64,
    pub(crate) first_update_pending: bool,
    pub(crate) poisoned: bool,
    pub(crate) current_event_host_execution: CurrentEventHostExecution,
    _event: PhantomData<fn() -> E>,
}

/// One mutable `SlayerScript` instance backed by immutable program tables.
pub struct SlayerRuntime<O, E, M, F>
where
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    pub(crate) state: RuntimeState<O, E>,
    pub(crate) modules: M,
    pub(crate) functions: F,
}

/// A short-lived, safely split executor over runtime state and both adapters.
///
/// Adapter callbacks explicitly lend both adapters back through
/// [`RuntimeContext`]. This is the ownership boundary that permits native
/// synchronous transition reentry without `unsafe`, `RefCell`, or delayed
/// command semantics.
pub struct RuntimeExecutor<'a, O, E, M, F> {
    pub(crate) state: &'a mut RuntimeState<O, E>,
    pub(crate) modules: &'a mut M,
    pub(crate) functions: &'a mut F,
}

impl<O, E, M, F> Deref for RuntimeExecutor<'_, O, E, M, F> {
    type Target = RuntimeState<O, E>;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl<O, E, M, F> DerefMut for RuntimeExecutor<'_, O, E, M, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
    }
}

impl<O, E, M, F> SlayerRuntime<O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Creates independent layer and transition-record state.
    #[must_use]
    pub fn new(
        program: Arc<SlayerProgram<O, E>>,
        modules: M,
        functions: F,
        current_event_host_execution: CurrentEventHostExecution,
    ) -> Self {
        let layers = program
            .layers()
            .iter()
            .zip(0..u32::MAX)
            .map(|(definition, index)| SequenceLayer::new(LayerId::new(index), definition))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let layer_driver = (0..layers.len())
            .map(|_| LayerDriverState::default())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            state: RuntimeState {
                program,
                layers,
                layer_driver,
                next_sequence_runtime_id: 0,
                next_event_runtime_id: 0,
                next_callback_instance_id: 0,
                first_update_pending: true,
                poisoned: false,
                current_event_host_execution,
                _event: PhantomData,
            },
            modules,
            functions,
        }
    }

    /// Returns the shared immutable program.
    #[must_use]
    pub fn program(&self) -> &SlayerProgram<O, E> {
        &self.state.program
    }

    /// Returns one read-only instance layer.
    #[must_use]
    pub fn layer(&self, id: LayerId) -> Option<&SequenceLayer<E>> {
        self.state.layers.get(id.index())
    }

    /// Returns the project module adapter.
    #[must_use]
    pub const fn modules(&self) -> &M {
        &self.modules
    }

    /// Returns the project function adapter.
    #[must_use]
    pub const fn functions(&self) -> &F {
        &self.functions
    }

    /// Returns mutable access to the project function adapter.
    ///
    /// This narrow integration surface lets a project drain typed outputs or
    /// update adapter-owned services without exposing mutable layer state.
    #[must_use]
    pub const fn functions_mut(&mut self) -> &mut F {
        &mut self.functions
    }

    /// Returns whether adapter failure may have left external effects partial.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.state.poisoned
    }

    /// Synchronously routes a typed event to its owning module.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] when routing, a callback transition, or an
    /// adapter fails.
    pub fn dispatch_typed(
        &mut self,
        event: &M::TypedEvent,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.executor().dispatch_typed(event)
    }

    /// Synchronously routes a targeted or fanout custom event.
    ///
    /// # Errors
    ///
    /// Unknown targets and zero-recipient fanout are native-compatible no-ops.
    pub fn dispatch_custom(
        &mut self,
        target: CustomDispatchTarget,
        event: &M::CustomEvent,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.executor().dispatch_custom(target, event)
    }

    /// Applies a transition outside a callback-bit deferral region.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] for invalid identifiers, adapter failure, or ID
    /// exhaustion. Native-compatible blocked outcomes are returned explicitly.
    pub fn trans(
        &mut self,
        layer: LayerId,
        request: TransitionRequest,
    ) -> Result<TransitionOutcome, RuntimeError<M::Error, F::Error>> {
        self.executor().request_transition(
            layer,
            request,
            None,
            CallbackRegistrationTarget::Layer {
                layer_index: layer.index(),
            },
        )
    }

    /// Applies the public indexed `SwitchStateOnLayer` boundary.
    ///
    /// Invalid IDs, including [`crate::StateId::NONE`], are native-compatible
    /// no-ops. State actions may enqueue further switches into the bounded
    /// per-layer FIFO.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::UnknownLayer`] when `layer` is outside the
    /// compiled layer table, or [`RuntimeError::Function`] when the adapter
    /// fails while evaluating the block guard, refreshing layer metadata, or
    /// running a state ENTER/EXIT action.
    pub fn switch_state(
        &mut self,
        layer: LayerId,
        state: crate::StateId,
        force: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.executor()
            .request_state_change(layer, state, force, None)
    }

    /// Returns the current signed state ID, or `-1` before installation.
    #[must_use]
    pub fn current_state(&self, layer: LayerId) -> Option<crate::StateId> {
        self.state
            .layer_driver
            .get(layer.index())
            .map(|driver| driver.state.current)
    }

    /// Runs the native-proved update order through module update.
    ///
    /// Deferred interval callbacks remain queued until
    /// [`Self::flush_interval_callbacks`] is called by the owning schedule.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] before mutation for time overflow, or poisons on
    /// adapter failure.
    pub fn update(
        &mut self,
        delta: DurationSeconds,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.executor().update(delta)
    }

    /// Completes the callback-flush schedule boundary.
    ///
    /// Fixed current-layer callbacks execute directly during [`Self::update`].
    /// This boundary first processes retained callback nodes and then drains
    /// their 12-byte-equivalent FIFO records in native phase order.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] when callback execution or a callback-requested
    /// transition fails.
    pub fn flush_interval_callbacks(&mut self) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.executor().flush_interval_callbacks()
    }

    const fn executor(&mut self) -> RuntimeExecutor<'_, O, E, M, F> {
        RuntimeExecutor {
            state: &mut self.state,
            modules: &mut self.modules,
            functions: &mut self.functions,
        }
    }
}

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    fn dispatch_typed(
        &mut self,
        event: &M::TypedEvent,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.ensure_healthy()?;
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
            let result = modules.dispatch_typed(event, functions, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.module_failure(error);
        }
        Ok(())
    }

    fn dispatch_custom(
        &mut self,
        target: CustomDispatchTarget,
        event: &M::CustomEvent,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.ensure_healthy()?;
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
            let result = match target {
                CustomDispatchTarget::Module(module) => {
                    modules.dispatch_custom_targeted(module, event, functions, &mut context)
                }
                CustomDispatchTarget::Fanout => {
                    modules.dispatch_custom_fanout(event, functions, &mut context)
                }
            };
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.module_failure(error);
        }
        Ok(())
    }

    fn update(&mut self, delta: DurationSeconds) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.ensure_healthy()?;
        self.prevalidate_update(delta)?;
        if self.first_update_pending {
            self.dispatch_first_update()?;
            self.first_update_pending = false;
        }
        for layer_index in 0..self.layers.len() {
            if self.layers[layer_index].kind() == LayerKind::Normal {
                let scaled_delta = delta.get() * self.layers[layer_index].playback_rate().get();
                self.update_layer(layer_index, scaled_delta)?;
            }
        }
        for layer_index in 0..self.layers.len() {
            if self.layers[layer_index].kind() == LayerKind::Auxiliary {
                self.update_auxiliary_layer(layer_index, delta.get())?;
            }
        }
        self.destroy_stopped_retained_callbacks()?;
        for layer_index in 0..self.layers.len() {
            if self.layers[layer_index].kind() == LayerKind::Normal {
                let scaled_delta = delta.get() * self.layers[layer_index].playback_rate().get();
                if scaled_delta > 0.0 {
                    self.advance_transition_records(layer_index, scaled_delta)?;
                    self.layers[layer_index].update_weights(scaled_delta);
                    self.compact_layer_records(layer_index);
                }
            }
        }
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
            let result = modules.update(delta, functions, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.module_failure(error);
        }
        Ok(())
    }

    fn flush_interval_callbacks(&mut self) -> Result<(), RuntimeError<M::Error, F::Error>> {
        self.ensure_healthy()?;
        self.flush_deferred_callback_queue()
    }

    fn dispatch_first_update(&mut self) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
            let result = modules.dispatch_on_start(OnStart, functions, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.module_failure(error);
        }
        Ok(())
    }

    fn prevalidate_update(
        &self,
        delta: DurationSeconds,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        for layer in &self.layers {
            let update_delta = if layer.kind() == LayerKind::Auxiliary {
                delta.get()
            } else {
                delta.get() * layer.playback_rate().get()
            };
            if !update_delta.is_finite() {
                return Err(RuntimeError::TimeOverflow { layer: layer.id() });
            }
            if update_delta == 0.0 {
                continue;
            }
            if layer.kind() == LayerKind::Normal
                && layer.current().is_some()
                && (!(layer.cumulative_time_seconds + update_delta).is_finite()
                    || !(layer.current_time_seconds + update_delta).is_finite())
            {
                return Err(RuntimeError::TimeOverflow { layer: layer.id() });
            }
            let newest_is_exiting = layer.records.last().is_some_and(|record| record.exiting);
            for (record_index, record) in layer.records.iter().enumerate() {
                let advances_time =
                    layer.kind() == LayerKind::Auxiliary || (update_delta > 0.0 && !record.exiting);
                let advances_transition_elapsed = layer.kind() == LayerKind::Normal
                    && update_delta > 0.0
                    && ((!record.exiting
                        && record.transition_duration_seconds > 0.0
                        && record.transition_elapsed_seconds < record.transition_duration_seconds)
                        || (!newest_is_exiting
                            && layer.records.len() > 1
                            && record_index == 0
                            && record.transition_duration_seconds > 0.0
                            && record.transition_duration_seconds
                                < crate::DEFAULT_OUTGOING_TRANSITION_SECONDS));
                if !record.cumulative_time_seconds.is_finite()
                    || !record.current_time_seconds.is_finite()
                    || (advances_time
                        && (!(record.cumulative_time_seconds + update_delta).is_finite()
                            || !(record.current_time_seconds + update_delta).is_finite()))
                    || (advances_transition_elapsed
                        && !(record.transition_elapsed_seconds + update_delta).is_finite())
                {
                    return Err(RuntimeError::TimeOverflow { layer: layer.id() });
                }
                if (layer.kind() == LayerKind::Auxiliary || update_delta > 0.0)
                    && record
                        .embedded_primary_event_tracks
                        .iter()
                        .chain(&record.embedded_payload_event_tracks)
                        .any(|runtime| !(runtime.fade_elapsed_seconds + update_delta).is_finite())
                {
                    return Err(RuntimeError::TimeOverflow { layer: layer.id() });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn map_advance_error(
        &mut self,
        layer: LayerId,
        error: SequenceAdvanceError,
    ) -> RuntimeError<M::Error, F::Error> {
        self.poisoned = true;
        match error {
            SequenceAdvanceError::TimeOverflow => RuntimeError::TimeOverflow { layer },
        }
    }

    pub(crate) fn ensure_healthy(&self) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if self.poisoned {
            Err(RuntimeError::Poisoned)
        } else {
            Ok(())
        }
    }

    pub(crate) fn module_failure<T>(
        &mut self,
        error: M::Error,
    ) -> Result<T, RuntimeError<M::Error, F::Error>> {
        self.poisoned = true;
        Err(RuntimeError::Module(error))
    }

    pub(crate) fn function_failure<T>(
        &mut self,
        error: F::Error,
    ) -> Result<T, RuntimeError<M::Error, F::Error>> {
        self.poisoned = true;
        Err(RuntimeError::Function(error))
    }
}

/// Why deterministic `SlayerScript` execution cannot continue.
#[derive(Debug, Error)]
pub enum RuntimeError<ME, FE>
where
    ME: StdError + Send + Sync + 'static,
    FE: StdError + Send + Sync + 'static,
{
    /// Project module routing failed.
    #[error("SlayerScript module adapter failed: {0}")]
    Module(#[source] ME),
    /// Project function execution failed.
    #[error("SlayerScript function adapter failed: {0}")]
    Function(#[source] FE),
    /// A prior adapter failure may have left external effects partial.
    #[error("SlayerScript runtime is poisoned by a prior adapter failure")]
    Poisoned,
    /// A request names no compiled layer.
    #[error("unknown SlayerScript layer {layer:?}")]
    UnknownLayer { layer: LayerId },
    /// A transition names no compiled sequence.
    #[error("unknown SlayerScript sequence {sequence:?}")]
    UnknownSequence { sequence: SequenceId },
    /// The compiler did not bind a sequence to the requested layer.
    #[error("SlayerScript sequence {sequence:?} is not bound to layer {layer:?}")]
    SequenceNotBoundToLayer {
        layer: LayerId,
        sequence: SequenceId,
    },
    /// Finite inputs would overflow native `f32` time state.
    #[error("SlayerScript layer {layer:?} time would overflow")]
    TimeOverflow { layer: LayerId },
    /// Externally driven playback has no installed typed host adapter.
    #[error("externally driven event runtime {runtime_id:?} has no host adapter")]
    MissingExternalDriveAdapter { runtime_id: EventRuntimeId },
    /// The typed host returned a non-finite playback increment.
    #[error("externally driven event runtime {runtime_id:?} returned a non-finite increment")]
    InvalidExternalPlaybackIncrement { runtime_id: EventRuntimeId },
    /// Executable primary tracks require the instance functions event host.
    #[error("current-layer executable event tracks have no instance function host")]
    MissingCurrentEventHost,
    /// Current-layer host step replacement must remain finite.
    #[error("current-layer event step for {layer:?} was not finite")]
    InvalidCurrentEventStep { layer: LayerId },
    /// A synchronous transition was requested from native's iterator-unsafe
    /// stop-path EXIT reentrancy hole.
    #[error("SlayerScript transition reentered unsafe stop-path EXIT on layer {layer:?}")]
    UnsafeStopExitReentry { layer: LayerId },
    /// Safe Rust cannot mutably reenter the callback object already executing.
    #[error("SlayerScript callback object {callback:?} reentered itself mutably")]
    UnsafeCallbackObjectReentry { callback: CallbackRuntimeId },
    /// Sequence-callback registration ran without an active layer/root.
    #[error("SlayerScript interval callback registration has no active sequence root")]
    NoActiveCallbackRegistrationTarget,
    /// Monotonic native runtime identifiers were exhausted.
    #[error("SlayerScript runtime identifier space exhausted")]
    RuntimeIdExhausted,
}
