//! Finalized state registrations and per-layer state-change semantics.

use arrayvec::ArrayVec;

use thiserror::Error;

use crate::{
    FunctionAdapter, LayerId, ModuleAdapter, RuntimeContext, RuntimeError, RuntimeExecutor,
    SequenceId, SlayerScriptLiteral, StateId, runtime::CallbackRegistrationTarget,
};

/// Opaque handle written by native state finalization.
///
/// A binding identifies one registration slot, not a statically assigned
/// [`StateId`]. Resolve it through the finalized [`StateTable`] before a module
/// mutates live data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct StateBinding(u32);

/// Two compiler ordering words passed by native state registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRegistrationMetadata {
    pub primary: i32,
    pub secondary: i32,
}

impl StateRegistrationMetadata {
    /// Native generated registrants use `(-1, -1)` and preserve the compiler's
    /// supplied registration order.
    pub const PRESERVE_REGISTRATION_ORDER: Self = Self {
        primary: -1,
        secondary: -1,
    };
}

/// One compiler-ordered state record after finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDefinition<O> {
    literal: SlayerScriptLiteral,
    registration_metadata: StateRegistrationMetadata,
    actions: Box<[O]>,
}

impl<O> StateDefinition<O> {
    #[must_use]
    pub const fn literal(&self) -> SlayerScriptLiteral {
        self.literal
    }

    #[must_use]
    pub const fn actions(&self) -> &[O] {
        &self.actions
    }

    /// Returns the exact compiler ordering words retained from registration.
    #[must_use]
    pub const fn registration_metadata(&self) -> StateRegistrationMetadata {
        self.registration_metadata
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PendingStateDefinition<O> {
    literal: SlayerScriptLiteral,
    metadata: StateRegistrationMetadata,
    actions: Box<[O]>,
}

/// Compiler-facing ordered state registrant collection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateTableBuilder<O> {
    registrations: Vec<PendingStateDefinition<O>>,
}

impl<O> StateTableBuilder<O> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            registrations: Vec::new(),
        }
    }

    /// Registers one literal-keyed state and returns its unresolved binding.
    ///
    /// # Errors
    ///
    /// Returns [`StateTableError::TooManyStates`] when the builder already
    /// holds more registrations than a `u32` binding index can address.
    pub fn register_state(
        &mut self,
        literal: SlayerScriptLiteral,
        metadata: StateRegistrationMetadata,
        actions: impl Into<Box<[O]>>,
    ) -> Result<StateBinding, StateTableError> {
        let index = u32::try_from(self.registrations.len()).map_err(|_| {
            StateTableError::TooManyStates {
                count: self.registrations.len(),
            }
        })?;
        self.registrations.push(PendingStateDefinition {
            literal,
            metadata,
            actions: actions.into(),
        });
        Ok(StateBinding(index))
    }

    /// Finalizes signed state IDs using native's recovered ordering rule.
    ///
    /// When every primary key is `-1`, intrusive registration order is
    /// preserved. Otherwise the complete table is sorted by the primary key as
    /// an unsigned word, naturally placing `-1` after nonnegative keys.
    /// Duplicate keys are rejected because native's unstable sort does not
    /// prove deterministic relative order for equal keys.
    ///
    /// # Errors
    ///
    /// Returns [`StateTableError::TooManyStates`] when the registration count
    /// exceeds the signed [`StateId`] space, or
    /// [`StateTableError::DuplicateOrderingKey`] when two registrants share a
    /// primary ordering key and native's unstable sort would not fix their
    /// relative order.
    ///
    /// # Panics
    ///
    /// Panics if a resolved state index does not fit `i32`. The count check
    /// above makes that unreachable.
    pub fn finalize(self) -> Result<StateTable<O>, StateTableError> {
        if self.registrations.len() > i32::MAX as usize {
            return Err(StateTableError::TooManyStates {
                count: self.registrations.len(),
            });
        }
        let mut registrations = self
            .registrations
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>();
        if registrations
            .iter()
            .any(|(_, registration)| registration.metadata.primary != -1)
        {
            let mut keys = registrations
                .iter()
                .map(|(_, registration)| registration.metadata.primary.cast_unsigned())
                .collect::<Vec<_>>();
            keys.sort_unstable();
            if let Some(key) = keys
                .windows(2)
                .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
            {
                return Err(StateTableError::DuplicateOrderingKey { key });
            }
            registrations.sort_unstable_by_key(|(_, registration)| {
                registration.metadata.primary.cast_unsigned()
            });
        }
        let mut resolved_bindings = vec![StateId::NONE; registrations.len()];
        let mut definitions = Vec::with_capacity(registrations.len());
        for (state_index, (registration_index, registration)) in
            registrations.into_iter().enumerate()
        {
            let state = StateId::new(
                i32::try_from(state_index).expect("validated state count fits signed StateId"),
            );
            resolved_bindings[registration_index] = state;
            definitions.push(StateDefinition {
                literal: registration.literal,
                registration_metadata: registration.metadata,
                actions: registration.actions,
            });
        }
        Ok(StateTable {
            definitions: definitions.into_boxed_slice(),
            resolved_bindings: resolved_bindings.into_boxed_slice(),
        })
    }
}

/// Immutable finalized state table shared by all instances of one template.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateTable<O> {
    definitions: Box<[StateDefinition<O>]>,
    resolved_bindings: Box<[StateId]>,
}

impl<O> StateTable<O> {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            definitions: Box::default(),
            resolved_bindings: Box::default(),
        }
    }

    #[must_use]
    pub fn resolve(&self, binding: StateBinding) -> Option<StateId> {
        self.resolved_bindings.get(binding.0 as usize).copied()
    }

    #[must_use]
    pub fn definition(&self, state: StateId) -> Option<&StateDefinition<O>> {
        usize::try_from(state.get())
            .ok()
            .and_then(|index| self.definitions.get(index))
    }

    #[must_use]
    pub const fn definitions(&self) -> &[StateDefinition<O>] {
        &self.definitions
    }
}

/// Why a compiler-owned state table cannot be finalized faithfully.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateTableError {
    #[error("state table contains {count} registrations, exceeding signed StateId space")]
    TooManyStates { count: usize },
    #[error("state registration ordering key 0x{key:08x} is duplicated")]
    DuplicateOrderingKey { key: u32 },
}

/// Exact old/new/current-sequence payload for typed event CRC `0xd736574f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateChanged {
    pub old_state: StateId,
    pub new_state: StateId,
    pub current_sequence: Option<SequenceId>,
}

/// Exact action mask sent to a compiled state action root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct StateActionMask(u32);

impl StateActionMask {
    pub const ENTER: Self = Self(2);
    pub const EXIT: Self = Self(4);
    pub const UPDATE: Self = Self(8);

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Compiler-resolved selector stored by one compiled layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct StateUpdateSelectorId(i32);

impl StateUpdateSelectorId {
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// Exact per-layer selector event dispatched before ordinary state UPDATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateLayerStates {
    pub layer: LayerId,
    pub selector: StateUpdateSelectorId,
    pub input: bool,
}

impl UpdateLayerStates {
    pub const TYPE_UUID: uuid::Uuid =
        uuid::Uuid::from_u128(0x973b_edcb_bf6a_462c_aeac_6907_f2e2_b100);
    pub const CRC: u32 = 0xcea8_8482;
}

/// One immutable state action invocation.
#[derive(Debug, Clone, Copy)]
pub struct StateOperationInvocation<'a, O> {
    layer: LayerId,
    state: StateId,
    mask: StateActionMask,
    operation: &'a O,
}

impl<'a, O> StateOperationInvocation<'a, O> {
    pub(crate) const fn new(
        layer: LayerId,
        state: StateId,
        mask: StateActionMask,
        operation: &'a O,
    ) -> Self {
        Self {
            layer,
            state,
            mask,
            operation,
        }
    }

    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    #[must_use]
    pub const fn state(&self) -> StateId {
        self.state
    }

    #[must_use]
    pub const fn mask(&self) -> StateActionMask {
        self.mask
    }

    #[must_use]
    pub const fn operation(&self) -> &'a O {
        self.operation
    }
}

#[derive(Debug, Default)]
pub struct LayerStateRuntime {
    pub(crate) current: StateId,
    pub(crate) previous: StateId,
    pub(crate) queue: ArrayVec<StateId, 10>,
    pub(crate) action_mask: Option<StateActionMask>,
    pub(crate) deferred_update_state: Option<StateId>,
}

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    /// Applies the public indexed state-switch boundary.
    pub(crate) fn request_state_change(
        &mut self,
        layer: LayerId,
        target: StateId,
        force: bool,
        callback_guard_layer: Option<LayerId>,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer_index = layer.index();
        if layer_index >= self.layers.len() {
            return Err(RuntimeError::UnknownLayer { layer });
        }
        if self.program.states().definition(target).is_none() {
            return Ok(());
        }
        if callback_guard_layer == Some(layer) {
            self.layer_driver[layer_index].state.deferred_update_state = Some(target);
            return Ok(());
        }
        self.enqueue_and_drain_state(layer_index, target, force)
    }

    pub(crate) fn apply_pending_state(
        &mut self,
        layer_index: usize,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if let Some(target) = self.layer_driver[layer_index]
            .state
            .deferred_update_state
            .take()
        {
            self.request_state_change(self.layers[layer_index].id, target, false, None)?;
        }
        Ok(())
    }

    fn enqueue_and_drain_state(
        &mut self,
        layer_index: usize,
        target: StateId,
        force: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer = self.layers[layer_index].id;
        let current = self.layer_driver[layer_index].state.current;
        let blocked = match self.functions.state_change_blocked(layer, current, target) {
            Ok(blocked) => blocked,
            Err(error) => return self.function_failure(error),
        };
        if blocked || (!force && current == target) {
            return Ok(());
        }
        if self.layer_driver[layer_index]
            .state
            .queue
            .try_push(target)
            .is_err()
        {
            return Ok(());
        }
        if self.layer_driver[layer_index]
            .state
            .action_mask
            .is_some_and(|mask| mask == StateActionMask::ENTER || mask == StateActionMask::EXIT)
        {
            return Ok(());
        }
        while !self.layer_driver[layer_index].state.queue.is_empty() {
            let next = self.layer_driver[layer_index].state.queue.remove(0);
            self.apply_state_change(layer_index, next)?;
        }
        Ok(())
    }

    fn apply_state_change(
        &mut self,
        layer_index: usize,
        next: StateId,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer = self.layers[layer_index].id;
        let old = self.layer_driver[layer_index].state.current;
        let current_sequence = self.layers[layer_index].current();
        if !old.is_none() {
            self.execute_state_actions(layer_index, old, StateActionMask::EXIT)?;
        }
        self.layer_driver[layer_index].state.previous = old;
        self.layer_driver[layer_index].state.current = next;
        if let Err(error) = self.functions.refresh_state_layer_metadata(layer) {
            return self.function_failure(error);
        }
        self.execute_state_actions(layer_index, next, StateActionMask::ENTER)?;
        let event = StateChanged {
            old_state: old,
            new_state: next,
            current_sequence,
        };
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
            let result = functions.on_state_changed(event, modules, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.function_failure(error);
        }
        Ok(())
    }

    fn execute_state_actions(
        &mut self,
        layer_index: usize,
        state_id: StateId,
        mask: StateActionMask,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let outer_mask = self.layer_driver[layer_index]
            .state
            .action_mask
            .replace(mask);
        let program = std::sync::Arc::clone(&self.program);
        let definition = program
            .states()
            .definition(state_id)
            .expect("validated state ID must remain compiled");
        let layer = self.layers[layer_index].id;
        for operation in definition.actions() {
            let (result, failure) = {
                let RuntimeExecutor {
                    state,
                    modules,
                    functions,
                } = self;
                let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false)
                    .with_callback_registration_target(CallbackRegistrationTarget::Layer {
                        layer_index,
                    });
                let result = functions.execute_state_operation(
                    StateOperationInvocation::new(layer, state_id, mask, operation),
                    modules,
                    &mut context,
                );
                (result, context.take_failure())
            };
            if let Some(error) = failure {
                self.layer_driver[layer_index].state.action_mask = outer_mask;
                return Err(error);
            }
            if let Err(error) = result {
                self.layer_driver[layer_index].state.action_mask = outer_mask;
                return self.function_failure(error);
            }
        }
        self.layer_driver[layer_index].state.action_mask = outer_mask;
        Ok(())
    }

    pub(crate) fn update_current_state(
        &mut self,
        layer_index: usize,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let layer = self.layers[layer_index].id;
        if let Some(selector) = self
            .program
            .layer(layer)
            .expect("validated runtime layer must have a definition")
            .state_update_selector()
        {
            let event = UpdateLayerStates {
                layer,
                selector,
                input: false,
            };
            let (result, failure) = {
                let RuntimeExecutor {
                    state,
                    modules,
                    functions,
                } = self;
                let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
                let result = functions.dispatch_update_layer_states(event, modules, &mut context);
                (result, context.take_failure())
            };
            if let Some(error) = failure {
                return Err(error);
            }
            let selected = match result {
                Ok(selected) => selected,
                Err(error) => return self.function_failure(error),
            };
            if selected {
                let (result, failure) = {
                    let RuntimeExecutor {
                        state,
                        modules,
                        functions,
                    } = self;
                    let mut context = RuntimeContext::<O, E, M, F>::new(state, None, false);
                    let result =
                        functions.execute_selected_state_update(event, modules, &mut context);
                    (result, context.take_failure())
                };
                if let Some(error) = failure {
                    return Err(error);
                }
                if let Err(error) = result {
                    return self.function_failure(error);
                }
                return Ok(());
            }
        }

        let current = self.layer_driver[layer_index].state.current;
        if !current.is_none() {
            self.execute_state_actions(layer_index, current, StateActionMask::UPDATE)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(crc: u32) -> SlayerScriptLiteral {
        SlayerScriptLiteral { crc }
    }

    #[test]
    fn all_negative_primary_keys_preserve_registration_order_and_bindings() {
        let mut builder = StateTableBuilder::<()>::new();
        let first = builder
            .register_state(
                literal(0x651a_00e5),
                StateRegistrationMetadata::PRESERVE_REGISTRATION_ORDER,
                [],
            )
            .unwrap();
        let second = builder
            .register_state(
                literal(0xf59f_4904),
                StateRegistrationMetadata::PRESERVE_REGISTRATION_ORDER,
                [],
            )
            .unwrap();

        let table = builder.finalize().unwrap();

        assert_eq!(table.resolve(first), Some(StateId::new(0)));
        assert_eq!(table.resolve(second), Some(StateId::new(1)));
        assert_eq!(table.definitions()[0].literal(), literal(0x651a_00e5));
        assert_eq!(table.definitions()[1].literal(), literal(0xf59f_4904));
    }

    #[test]
    fn explicit_primary_keys_sort_unsigned_and_rewrite_bindings() {
        let mut builder = StateTableBuilder::<()>::new();
        let unkeyed = builder
            .register_state(
                literal(3),
                StateRegistrationMetadata {
                    primary: -1,
                    secondary: 9,
                },
                [],
            )
            .unwrap();
        let later = builder
            .register_state(
                literal(2),
                StateRegistrationMetadata {
                    primary: 7,
                    secondary: -1,
                },
                [],
            )
            .unwrap();
        let earlier = builder
            .register_state(
                literal(1),
                StateRegistrationMetadata {
                    primary: 2,
                    secondary: 42,
                },
                [],
            )
            .unwrap();

        let table = builder.finalize().unwrap();

        assert_eq!(table.resolve(earlier), Some(StateId::new(0)));
        assert_eq!(table.resolve(later), Some(StateId::new(1)));
        assert_eq!(table.resolve(unkeyed), Some(StateId::new(2)));
        assert_eq!(
            table
                .definitions()
                .iter()
                .map(StateDefinition::literal)
                .collect::<Vec<_>>(),
            vec![literal(1), literal(2), literal(3)]
        );
        assert_eq!(table.definitions()[0].registration_metadata().secondary, 42);
    }

    #[test]
    fn duplicate_sort_keys_are_rejected_instead_of_claiming_native_tie_order() {
        let mut builder = StateTableBuilder::<()>::new();
        for crc in [1, 2] {
            builder
                .register_state(
                    literal(crc),
                    StateRegistrationMetadata {
                        primary: 4,
                        secondary: -1,
                    },
                    [],
                )
                .unwrap();
        }

        assert_eq!(
            builder.finalize().unwrap_err(),
            StateTableError::DuplicateOrderingKey { key: 4 }
        );
    }
}
