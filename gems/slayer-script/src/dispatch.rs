//! Adapter-facing transition reentry and direct event-routing targets.

use crate::{
    FunctionAdapter, IntervalCallbackDefinition, LayerId, ModuleAdapter, RuntimeError,
    RuntimeExecutor, RuntimeState, StateId, TransitionRequest, runtime::CallbackRegistrationTarget,
};

/// Typed first-update event with native literal CRC `0x8b372fca`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnStart;

impl OnStart {
    /// Native typed-event literal.
    pub const CRC: u32 = 0x8b37_2fca;
}

/// Routing for a custom event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomDispatchTarget {
    /// Deliver only to a named module.
    Module(crate::ModuleId),
    /// Deliver to every accepting module in stable registry order.
    Fanout,
}

/// Narrow adapter-facing capability for exact transition reentry.
///
/// The context owns only the engine state borrow. The callback explicitly
/// lends both adapters to [`Self::trans`], allowing native unguarded calls to
/// apply before the callback's next statement without `unsafe`, `RefCell`, or
/// a delayed command queue. Guarded non-null normal transitions still use the
/// native last-write-wins pending slot.
pub struct RuntimeContext<'a, O, E, M, F>
where
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    state: &'a mut RuntimeState<O, E>,
    callback_guard_layer: Option<LayerId>,
    callback_registration_target: Option<CallbackRegistrationTarget>,
    synchronous_stop_exit: bool,
    failure: Option<RuntimeError<M::Error, F::Error>>,
}

impl<O, E, M, F> RuntimeContext<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Applies one transition at this exact adapter call site.
    ///
    /// Native's synchronous stop-path EXIT reentrancy can revisit a mutable
    /// callback tree without iterator repair. The safe Rust runtime rejects
    /// only that pathological boundary and records a typed fatal error; every
    /// other unguarded/null/force transition is applied immediately.
    pub fn trans(
        &mut self,
        layer: LayerId,
        request: TransitionRequest,
        modules: &mut M,
        functions: &mut F,
    ) {
        if self.failure.is_some() {
            return;
        }
        if self.synchronous_stop_exit {
            self.state.poisoned = true;
            self.failure = Some(RuntimeError::UnsafeStopExitReentry { layer });
            return;
        }
        let mut executor = RuntimeExecutor {
            state: self.state,
            modules,
            functions,
        };
        let callback_registration_target = self.callback_registration_target.unwrap_or_else(|| {
            CallbackRegistrationTarget::Layer {
                layer_index: layer.index(),
            }
        });
        if let Err(error) = executor.request_transition(
            layer,
            request,
            self.callback_guard_layer,
            callback_registration_target,
        ) {
            self.failure = Some(error);
        }
    }

    /// Requests a signed state switch at this exact adapter call site.
    ///
    /// Calls made in the current-time guarded lane populate its separate
    /// pending `StateId`; state ENTER/EXIT reentrancy uses the bounded state
    /// FIFO owned by the layer runtime.
    pub fn switch_state(
        &mut self,
        layer: LayerId,
        state: StateId,
        force: bool,
        modules: &mut M,
        functions: &mut F,
    ) {
        if self.failure.is_some() {
            return;
        }
        let mut executor = RuntimeExecutor {
            state: self.state,
            modules,
            functions,
        };
        if let Err(error) =
            executor.request_state_change(layer, state, force, self.callback_guard_layer)
        {
            self.failure = Some(error);
        }
    }

    /// Registers one owned sequence callback in the currently executing root.
    ///
    /// Native sequence actions target the layer-owned retained map. Auxiliary
    /// record actions temporarily redirect the same registration entry point to
    /// that record's private runtime map. Equal wrapping runtime IDs are silent
    /// no-ops after the owned definition has been constructed.
    pub fn register_interval_callback(
        &mut self,
        definition: &IntervalCallbackDefinition<E>,
        modules: &mut M,
        functions: &mut F,
    ) {
        if self.failure.is_some() {
            return;
        }
        let Some(target) = self.callback_registration_target else {
            self.state.poisoned = true;
            self.failure = Some(RuntimeError::NoActiveCallbackRegistrationTarget);
            return;
        };
        let mut executor = RuntimeExecutor {
            state: self.state,
            modules,
            functions,
        };
        if let Err(error) = executor.register_sequence_callback(definition, target) {
            self.failure = Some(error);
        }
    }

    pub(crate) const fn new(
        state: &mut RuntimeState<O, E>,
        callback_guard_layer: Option<LayerId>,
        synchronous_stop_exit: bool,
    ) -> RuntimeContext<'_, O, E, M, F> {
        RuntimeContext {
            state,
            callback_guard_layer,
            callback_registration_target: None,
            synchronous_stop_exit,
            failure: None,
        }
    }

    pub(crate) const fn with_callback_registration_target(
        mut self,
        target: CallbackRegistrationTarget,
    ) -> Self {
        self.callback_registration_target = Some(target);
        self
    }

    pub(crate) const fn take_failure(&mut self) -> Option<RuntimeError<M::Error, F::Error>> {
        self.failure.take()
    }
}
