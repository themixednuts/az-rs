//! Typed source schemas and a deterministic `SlayerScript` executor.
//!
//! `SlayerScript` authoring data is compiled into immutable [`SlayerProgram`]
//! tables. Each [`SlayerRuntime`] owns only its mutable layer and dispatch
//! state; project behavior enters through typed module and function adapters.
//! The hot path never interprets reflection data or an untyped property bag.
//! This gem deliberately registers no asset product: the operation type is a
//! project contract, so the concrete project owns honest program-product
//! serialization and constructs a validated [`SlayerProgram`].
//!
//! The Rust implementation models validated runtime semantics with owned data;
//! it does not reproduce private native storage layout.

#![forbid(unsafe_code)]

mod adapters;
mod catalog;
mod current_event;
mod current_event_callbacks;
mod current_event_host;
mod dispatch;
mod event_execution;
mod event_track;
mod ids;
mod literal;
mod program;
mod runtime;
mod sequence;
mod sequence_callbacks;
pub mod source;
mod state;
mod system_component;
mod transition;

pub use adapters::{
    CurrentEventHost, CurrentEventStartRequest, CurrentEventStepRequest, CurrentEventStopRequest,
    CurrentEventUpdateRequest, ExecutableEventChannel, FunctionAdapter, IntervalCallbackBinding,
    IntervalCallbackInvocation, IntervalCallbackScope, ModuleAdapter, OperationInvocation,
    TransitionGuard,
};
pub use catalog::{ScriptTemplateRegistry, ScriptTemplateRegistryError};
pub(crate) use current_event_host::CurrentEventRoute;
pub use dispatch::{CustomDispatchTarget, OnStart, RuntimeContext};
pub use event_track::{
    BoundEventProperties, CurrentEventTrackRuntime, EventCallbackPhase, EventIntervalDefinition,
    EventRootDefinition, EventTrackDefinition, EventTrackRuntime, EventTrackScalar,
    EventTrackValidationError, ExecutableEventId, ExternalDriveRouteKey, ExternalPlaybackRequest,
    IntervalCallbackDefinition,
};
pub(crate) use event_track::{CurrentEventCallbackRuntime, CurrentEventCallbackState};
pub use ids::{
    CallbackAuthoredId, CallbackRuntimeId, EventRuntimeId, LayerId, ModuleId, SequenceId,
    SequenceRuntimeId, StateId,
};
pub use literal::{SlayerScriptEditLiteral, SlayerScriptLiteral, SlayerScriptName};
pub use program::{
    AuthoredEventGroupCount, AuthoredFrames, AuthoredFramesError, DurationSeconds,
    DurationSecondsError, ExecutableEventChannelCount, ExecutableEventChannelCountError,
    LayerDefinition, LayerKind, LayerPlaybackRate, LayerPlaybackRateError,
    PayloadEventTrackDefinition, ProgramValidationError, SequenceDefinition, SlayerProgram,
};
pub use runtime::{CurrentEventHostExecution, RuntimeError, SlayerRuntime};
pub(crate) use runtime::{RuntimeExecutor, RuntimeState};
pub use sequence::{
    DEFAULT_OUTGOING_TRANSITION_SECONDS, INFINITE_SEQUENCE_DURATION_SECONDS,
    MAX_TRANSITION_NESTING, ParentSequenceChanged, ParentSequenceContext, ResolvedParentId,
    SequenceActionMask, SequenceChanged, SequenceLayer, SequencePhase, SequenceTransitionRuntime,
    TransitionOutcome, TransitionRequest,
};
pub use source::{
    EntityEvent, EntityEvents, OpacityEvent, RotationEvent, SlayerScriptData,
    SlayerScriptDataContainer, SlayerScriptEditCrc, SlayerScriptSource,
};
pub use state::{
    StateActionMask, StateBinding, StateChanged, StateDefinition, StateOperationInvocation,
    StateRegistrationMetadata, StateTable, StateTableBuilder, StateTableError,
    StateUpdateSelectorId, UpdateLayerStates,
};
pub use system_component::{
    SLAYER_SCRIPT_SYSTEM_COMPONENT_TYPE_UUID, SlayerScriptPlugin, SlayerScriptSystemComponent,
    prefab_types, register_source_container, types,
};

use az_gem_contract::{Contribution, GemContext, contribution};

// This package implements both of its gem's contributions, so each block names
// which one it is with a bare token.

/// The gem's `package` contribution: the AZ type identity every runtime and
/// authoring host reads.
///
/// Sealing is privacy: the generated `package_contribution` is the only way in.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::rtti::AzTypeRegistration>()
            .register_many(types());
    }
}

/// The gem's `prefab-types` contribution: the reflected authoring types an
/// asset worker needs to compile prefabs.
///
/// These ten types are this gem's own, and this is the one contribution that
/// claims them. A reflected type gets exactly one owning contribution, and the
/// owner is the gem that defines it.
struct PrefabTypes;

#[contribution(prefab_types)]
impl Contribution for PrefabTypes {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_prefab::PrefabType>()
            .register_many(prefab_types());
    }
}
