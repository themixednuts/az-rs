#![allow(dead_code)]

use std::sync::Arc;

use az_gem_slayer_script::{
    AuthoredFrames, CallbackAuthoredId, CallbackRuntimeId, CurrentEventHost,
    CurrentEventHostExecution, CurrentEventStartRequest, CurrentEventStepRequest,
    CurrentEventStopRequest, CurrentEventUpdateRequest, CustomDispatchTarget, DurationSeconds,
    EventCallbackPhase, ExecutableEventId, ExternalPlaybackRequest, FunctionAdapter,
    IntervalCallbackBinding, IntervalCallbackDefinition, IntervalCallbackInvocation,
    LayerDefinition, LayerId, ModuleAdapter, ModuleId, OnStart, OperationInvocation,
    ParentSequenceChanged, RuntimeContext, SequenceActionMask, SequenceChanged, SequenceDefinition,
    SequenceId, SequencePhase, SlayerProgram, SlayerRuntime, StateActionMask, StateChanged,
    StateId, StateOperationInvocation, StateTable, TransitionGuard, TransitionRequest,
    UpdateLayerStates,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedEvent {
    pub owner: ModuleId,
    pub value: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomEvent(pub u8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestOperation {
    Mark(&'static str, SequencePhase),
    RecordMask(&'static str),
    Trans {
        on: SequencePhase,
        next: Option<SequenceId>,
        transition_frames: f32,
        initial_time_frames: f32,
        force: bool,
    },
    SwitchState {
        on: SequencePhase,
        next: StateId,
        force: bool,
    },
    StateMark(&'static str, StateActionMask),
    StateSwitch {
        on: StateActionMask,
        next: StateId,
        force: bool,
    },
    RegisterCallback {
        on: SequencePhase,
        authored_id: u32,
        label: &'static str,
        start_seconds: f32,
        end_seconds: f32,
        may_defer: bool,
    },
    RegisterTransitionCallback {
        on: SequencePhase,
        authored_id: u32,
        label: &'static str,
        start_seconds: f32,
        end_seconds: f32,
        transition_on: EventCallbackPhase,
        next: Option<SequenceId>,
    },
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestCallback {
    pub label: &'static str,
    pub transition: Option<SequenceId>,
    pub transition_on: Option<EventCallbackPhase>,
}

impl TestCallback {
    pub const fn direct(label: &'static str) -> Self {
        Self {
            label,
            transition: None,
            transition_on: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogEntry {
    OnStart(ModuleId),
    Operation(&'static str, SequencePhase, SequenceActionMask),
    TransitionOperation(Option<SequenceId>),
    Changed(SequenceChanged),
    ParentChanged(ParentSequenceChanged),
    StateOperation(&'static str, StateId, StateActionMask),
    StateMetadataRefreshed(LayerId),
    StateChanged(StateChanged),
    Callback(&'static str, EventCallbackPhase, f32),
    Typed(ModuleId, u8),
    Custom(ModuleId, u8),
    ModuleUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TestFunctionError {
    #[error("operation failed")]
    OperationFailed,
    #[error("external playback failed")]
    ExternalPlaybackFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TestModuleError {
    #[error("unknown custom-event target")]
    UnknownTarget,
    #[error("fanout has no accepting module")]
    NoRecipients,
    #[error("typed event has no owner")]
    NoTypedOwner,
}

// Independent host-behavior switches on a test double: each is read at exactly
// one adapter call site and any combination of them is a valid fixture, so no
// enum models them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct TestFunctions {
    pub log: Vec<LogEntry>,
    pub reject_target: Option<SequenceId>,
    pub external_increment: Option<f32>,
    pub external_requests: Vec<ExternalPlaybackRequest>,
    pub operation_transition_budget: Option<usize>,
    pub module_outputs: Vec<u8>,
    pub lifecycle_blocked: bool,
    pub current_event_gate_open: bool,
    pub current_event_step: Option<f32>,
    pub current_event_starts: Vec<CurrentEventStartRequest>,
    pub current_event_stops: Vec<CurrentEventStopRequest>,
    pub current_event_updates: Vec<CurrentEventUpdateRequest>,
    pub current_event_gates: Vec<ExecutableEventId>,
    pub current_event_steps: Vec<CurrentEventStepRequest>,
    pub current_event_callbacks: Vec<(CallbackRuntimeId, EventCallbackPhase, f32)>,
    pub callback_lifecycle: Vec<(&'static str, &'static str)>,
    pub state_blocked: bool,
    pub capture_parent_events: bool,
    pub transition_preflight_log: Vec<&'static str>,
}

impl CurrentEventHost<TestCallback> for TestFunctions {
    type Error = TestFunctionError;

    fn start_current_event(
        &mut self,
        request: CurrentEventStartRequest,
    ) -> Result<(), Self::Error> {
        self.current_event_starts.push(request);
        Ok(())
    }

    fn stop_current_event(&mut self, request: CurrentEventStopRequest) -> Result<(), Self::Error> {
        self.current_event_stops.push(request);
        Ok(())
    }

    fn update_current_event(
        &mut self,
        request: CurrentEventUpdateRequest,
    ) -> Result<(), Self::Error> {
        self.current_event_updates.push(request);
        Ok(())
    }

    fn current_event_gate(&mut self, event_id: ExecutableEventId) -> Result<bool, Self::Error> {
        self.current_event_gates.push(event_id);
        Ok(self.current_event_gate_open)
    }

    fn replace_current_event_step(
        &mut self,
        request: CurrentEventStepRequest,
    ) -> Result<f32, Self::Error> {
        self.current_event_steps.push(request);
        Ok(self.current_event_step.unwrap_or(request.delta_seconds))
    }
}

type TestContext<'a> = RuntimeContext<'a, TestOperation, TestCallback, TestModules, TestFunctions>;

impl FunctionAdapter<TestOperation, TestCallback, TestModules> for TestFunctions {
    type Error = TestFunctionError;

    fn current_event_host(
        &mut self,
    ) -> Option<&mut dyn CurrentEventHost<TestCallback, Error = Self::Error>> {
        Some(self)
    }

    fn execute_operation(
        &mut self,
        invocation: OperationInvocation<'_, TestOperation>,
        modules: &mut TestModules,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        match *invocation.operation() {
            TestOperation::Mark(label, on) => {
                if invocation.phase() == on {
                    self.log.push(LogEntry::Operation(
                        label,
                        invocation.phase(),
                        invocation.action_mask(),
                    ));
                }
            }
            TestOperation::RecordMask(label) => self.log.push(LogEntry::Operation(
                label,
                invocation.phase(),
                invocation.action_mask(),
            )),
            TestOperation::Trans {
                on,
                next,
                transition_frames,
                initial_time_frames,
                force,
            } => {
                if invocation.phase() != on {
                    return Ok(());
                }
                self.log.push(LogEntry::TransitionOperation(next));
                let may_transition = self
                    .operation_transition_budget
                    .is_none_or(|remaining| remaining > 0);
                if let Some(remaining) = &mut self.operation_transition_budget {
                    *remaining = remaining.saturating_sub(1);
                }
                if may_transition {
                    context.trans(
                        invocation.layer(),
                        TransitionRequest::new(
                            next,
                            frames(transition_frames),
                            frames(initial_time_frames),
                            force,
                        ),
                        modules,
                        self,
                    );
                }
            }
            TestOperation::SwitchState { on, next, force } => {
                if invocation.phase() == on {
                    context.switch_state(invocation.layer(), next, force, modules, self);
                }
            }
            TestOperation::RegisterCallback {
                on,
                authored_id,
                label,
                start_seconds,
                end_seconds,
                may_defer,
            } => {
                if invocation.phase() == on {
                    let definition = IntervalCallbackDefinition::new(
                        CallbackAuthoredId::new(authored_id),
                        seconds(start_seconds),
                        seconds(end_seconds),
                        TestCallback::direct(label),
                    )
                    .with_deferred_dispatch(may_defer);
                    context.register_interval_callback(&definition, modules, self);
                }
            }
            TestOperation::RegisterTransitionCallback {
                on,
                authored_id,
                label,
                start_seconds,
                end_seconds,
                transition_on,
                next,
            } => {
                if invocation.phase() == on {
                    let definition = IntervalCallbackDefinition::new(
                        CallbackAuthoredId::new(authored_id),
                        seconds(start_seconds),
                        seconds(end_seconds),
                        TestCallback {
                            label,
                            transition: next,
                            transition_on: Some(transition_on),
                        },
                    );
                    context.register_interval_callback(&definition, modules, self);
                }
            }
            TestOperation::StateMark(_, _) | TestOperation::StateSwitch { .. } => {}
            TestOperation::Fail => return Err(TestFunctionError::OperationFailed),
        }
        Ok(())
    }

    fn execute_interval_callback(
        &mut self,
        invocation: IntervalCallbackInvocation<'_, TestCallback>,
        modules: &mut TestModules,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.current_event_callbacks.push((
            invocation.callback_runtime_id(),
            invocation.phase(),
            invocation.delta_seconds(),
        ));
        let payload = invocation.payload();
        self.log.push(LogEntry::Callback(
            payload.label,
            invocation.phase(),
            invocation.delta_seconds(),
        ));
        if payload.transition_on == Some(invocation.phase()) {
            context.trans(
                LayerId::new(0),
                TransitionRequest::immediate(payload.transition),
                modules,
                self,
            );
        }
        Ok(())
    }

    fn bind_interval_callback(
        &mut self,
        _binding: IntervalCallbackBinding,
        callback: &mut TestCallback,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.callback_lifecycle.push(("bind", callback.label));
        Ok(())
    }

    fn initialize_interval_callback(
        &mut self,
        _binding: IntervalCallbackBinding,
        callback: &mut TestCallback,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.callback_lifecycle.push(("initialize", callback.label));
        Ok(())
    }

    fn finalize_interval_callback(
        &mut self,
        callback: &mut TestCallback,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.callback_lifecycle.push(("finalize", callback.label));
        Ok(())
    }

    fn execute_state_operation(
        &mut self,
        invocation: StateOperationInvocation<'_, TestOperation>,
        modules: &mut TestModules,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        match *invocation.operation() {
            TestOperation::StateMark(label, on) if invocation.mask() == on => {
                self.log.push(LogEntry::StateOperation(
                    label,
                    invocation.state(),
                    invocation.mask(),
                ));
            }
            TestOperation::StateSwitch { on, next, force } if invocation.mask() == on => {
                context.switch_state(invocation.layer(), next, force, modules, self);
            }
            _ => {}
        }
        Ok(())
    }

    fn dispatch_update_layer_states(
        &mut self,
        _event: UpdateLayerStates,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn execute_selected_state_update(
        &mut self,
        _event: UpdateLayerStates,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn state_change_blocked(
        &mut self,
        _layer: LayerId,
        _current: StateId,
        _next: StateId,
    ) -> Result<bool, Self::Error> {
        Ok(self.state_blocked)
    }

    fn refresh_state_layer_metadata(&mut self, layer: LayerId) -> Result<(), Self::Error> {
        self.log.push(LogEntry::StateMetadataRefreshed(layer));
        Ok(())
    }

    fn transition_application_blocked(&mut self) -> Result<bool, Self::Error> {
        self.transition_preflight_log.push("lifecycle");
        Ok(self.lifecycle_blocked)
    }

    fn blocks_transition_target(&mut self, guard: TransitionGuard) -> Result<bool, Self::Error> {
        self.transition_preflight_log.push("target");
        Ok(self.reject_target == Some(guard.next))
    }

    fn external_playback_increment(
        &mut self,
        request: ExternalPlaybackRequest,
    ) -> Result<Option<f32>, Self::Error> {
        self.external_requests.push(request);
        Ok(self.external_increment)
    }

    fn on_sequence_changed(
        &mut self,
        event: SequenceChanged,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.log.push(LogEntry::Changed(event));
        Ok(())
    }

    fn on_state_changed(
        &mut self,
        event: StateChanged,
        _modules: &mut TestModules,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        self.log.push(LogEntry::StateChanged(event));
        Ok(())
    }
}

#[derive(Debug)]
pub struct TestModules {
    pub runtime_owner: ModuleId,
    pub custom_acceptors: Vec<ModuleId>,
    pub dispatch_transition: Option<TransitionRequest>,
    pub runtime_state_switch: Option<StateId>,
    pub payload_event_owners: Vec<ModuleId>,
    pub payload_event_starts: Vec<CurrentEventStartRequest>,
    pub payload_event_stops: Vec<CurrentEventStopRequest>,
    pub payload_event_updates: Vec<CurrentEventUpdateRequest>,
}

impl Default for TestModules {
    fn default() -> Self {
        Self {
            runtime_owner: ModuleId::new(7),
            custom_acceptors: vec![ModuleId::new(2), ModuleId::new(5)],
            dispatch_transition: None,
            runtime_state_switch: None,
            payload_event_owners: vec![ModuleId::new(11)],
            payload_event_starts: Vec::new(),
            payload_event_stops: Vec::new(),
            payload_event_updates: Vec::new(),
        }
    }
}

impl CurrentEventHost<TestCallback> for TestModules {
    type Error = TestModuleError;

    fn start_current_event(
        &mut self,
        request: CurrentEventStartRequest,
    ) -> Result<(), Self::Error> {
        self.payload_event_starts.push(request);
        Ok(())
    }

    fn stop_current_event(&mut self, request: CurrentEventStopRequest) -> Result<(), Self::Error> {
        self.payload_event_stops.push(request);
        Ok(())
    }

    fn update_current_event(
        &mut self,
        request: CurrentEventUpdateRequest,
    ) -> Result<(), Self::Error> {
        self.payload_event_updates.push(request);
        Ok(())
    }

    fn current_event_gate(&mut self, _event_id: ExecutableEventId) -> Result<bool, Self::Error> {
        Ok(true)
    }

    fn replace_current_event_step(
        &mut self,
        request: CurrentEventStepRequest,
    ) -> Result<f32, Self::Error> {
        Ok(request.delta_seconds)
    }
}

impl ModuleAdapter<TestOperation, TestCallback, TestFunctions> for TestModules {
    type TypedEvent = TypedEvent;
    type CustomEvent = CustomEvent;
    type Error = TestModuleError;

    fn current_event_host(
        &mut self,
        owner: ModuleId,
    ) -> Option<&mut dyn CurrentEventHost<TestCallback, Error = Self::Error>> {
        if self.payload_event_owners.contains(&owner) {
            Some(self)
        } else {
            None
        }
    }

    fn dispatch_on_start(
        &mut self,
        _event: OnStart,
        functions: &mut TestFunctions,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        functions.log.push(LogEntry::OnStart(self.runtime_owner));
        if let Some(state) = self.runtime_state_switch.take() {
            context.switch_state(LayerId::new(0), state, false, self, functions);
        }
        Ok(())
    }

    fn dispatch_typed(
        &mut self,
        event: &Self::TypedEvent,
        functions: &mut TestFunctions,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        if event.value == u8::MAX {
            return Ok(());
        }
        if event.value == u8::MAX - 1 {
            self.payload_event_owners.clear();
            return Ok(());
        }
        functions
            .log
            .push(LogEntry::Typed(event.owner, event.value));
        if let Some(request) = self.dispatch_transition.take() {
            context.trans(LayerId::new(0), request, self, functions);
        }
        Ok(())
    }

    fn dispatch_custom_targeted(
        &mut self,
        target: ModuleId,
        event: &Self::CustomEvent,
        functions: &mut TestFunctions,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        if !self.custom_acceptors.contains(&target) {
            return Ok(());
        }
        functions.log.push(LogEntry::Custom(target, event.0));
        if let Some(request) = self.dispatch_transition.take() {
            context.trans(LayerId::new(0), request, self, functions);
        }
        Ok(())
    }

    fn dispatch_custom_fanout(
        &mut self,
        event: &Self::CustomEvent,
        functions: &mut TestFunctions,
        context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        for target in &self.custom_acceptors {
            functions.log.push(LogEntry::Custom(*target, event.0));
        }
        if let Some(request) = self.dispatch_transition.take() {
            context.trans(LayerId::new(0), request, self, functions);
        }
        Ok(())
    }

    fn dispatch_parent_sequence_changed(
        &mut self,
        event: ParentSequenceChanged,
        functions: &mut TestFunctions,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        if functions.capture_parent_events {
            functions.log.push(LogEntry::ParentChanged(event));
        }
        Ok(())
    }

    fn update(
        &mut self,
        _delta: DurationSeconds,
        functions: &mut TestFunctions,
        _context: &mut TestContext<'_>,
    ) -> Result<(), Self::Error> {
        functions.log.push(LogEntry::ModuleUpdate);
        Ok(())
    }
}

pub type TestRuntime = SlayerRuntime<TestOperation, TestCallback, TestModules, TestFunctions>;

pub fn seconds(value: f32) -> DurationSeconds {
    DurationSeconds::new(value).expect("fixture duration must be valid")
}

pub fn frames(value: f32) -> AuthoredFrames {
    AuthoredFrames::new(value).expect("fixture frames must be finite")
}

pub fn sequence(duration: f32, looping: bool) -> SequenceDefinition<TestOperation, TestCallback> {
    SequenceDefinition::new(seconds(duration), looping)
}

pub fn runtime(
    sequences: Vec<SequenceDefinition<TestOperation, TestCallback>>,
    layers: Vec<LayerDefinition>,
) -> TestRuntime {
    runtime_with(
        sequences,
        layers,
        TestModules::default(),
        TestFunctions::default(),
    )
}

pub fn runtime_with(
    sequences: Vec<SequenceDefinition<TestOperation, TestCallback>>,
    layers: Vec<LayerDefinition>,
    modules: TestModules,
    functions: TestFunctions,
) -> TestRuntime {
    runtime_with_states(sequences, layers, StateTable::empty(), modules, functions)
}

pub fn runtime_with_states(
    sequences: Vec<SequenceDefinition<TestOperation, TestCallback>>,
    layers: Vec<LayerDefinition>,
    states: StateTable<TestOperation>,
    modules: TestModules,
    functions: TestFunctions,
) -> TestRuntime {
    runtime_with_states_and_mode(
        sequences,
        layers,
        states,
        modules,
        functions,
        CurrentEventHostExecution::Enabled,
    )
}

pub fn runtime_with_states_and_mode(
    sequences: Vec<SequenceDefinition<TestOperation, TestCallback>>,
    layers: Vec<LayerDefinition>,
    states: StateTable<TestOperation>,
    modules: TestModules,
    functions: TestFunctions,
    current_event_host_execution: CurrentEventHostExecution,
) -> TestRuntime {
    let sequence_ids = (0..sequences.len())
        .map(|index| {
            SequenceId::new(u32::try_from(index).expect("fixture sequence count fits u32"))
        })
        .collect::<Vec<_>>();
    let layers = layers
        .into_iter()
        .map(|layer| layer.with_sequences(sequence_ids.clone()))
        .collect::<Vec<_>>();
    let program =
        SlayerProgram::new(sequences, layers, states).expect("fixture program must validate");
    SlayerRuntime::new(
        Arc::new(program),
        modules,
        functions,
        current_event_host_execution,
    )
}

pub const fn fanout() -> CustomDispatchTarget {
    CustomDispatchTarget::Fanout
}
