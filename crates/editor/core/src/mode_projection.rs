//! Typed lifecycle for editor mode projections.
//!
//! Mode modules own their state, actions, projection builders, and effects.
//! This module owns only installation and dependency-driven publication.

use std::any::{TypeId, type_name};

use gpui::{App, Global, Subscription};
use smallvec::SmallVec;
use thiserror::Error;

type InstallMode = fn(&mut App);
type PublishMode = fn(&mut App);
type ObserveInput = fn(&mut App) -> Subscription;

pub trait ModeProjectionSpec: Sized + 'static {
    type State: Default + Global;
    type Projection: Global;

    const NAME: &'static str;

    fn register_inputs(inputs: &mut ModeProjectionInputs);
    fn install_actions(cx: &mut App);
    fn project(state: &Self::State, cx: &App) -> Self::Projection;
}

#[derive(Default)]
pub struct ModeProjectionInputs {
    inputs: Vec<ModeProjectionInput>,
}

impl ModeProjectionInputs {
    pub(crate) fn depends_on<I: Global>(&mut self) {
        self.inputs.push(ModeProjectionInput::of::<I>());
    }
}

#[derive(Clone, Copy)]
struct ModeProjectionInput {
    type_id: TypeId,
    type_name: &'static str,
    observe: ObserveInput,
}

impl ModeProjectionInput {
    fn of<I: Global>() -> Self {
        Self {
            type_id: TypeId::of::<I>(),
            type_name: type_name::<I>(),
            observe: observe_input::<I>,
        }
    }
}

pub struct ModeProjectionRegistration {
    type_id: TypeId,
    name: &'static str,
    inputs: Vec<ModeProjectionInput>,
    install: InstallMode,
    publish: PublishMode,
}

impl ModeProjectionRegistration {
    pub(crate) fn for_spec<S: ModeProjectionSpec>() -> Result<Self, ModeProjectionRegistrationError>
    {
        let mut inputs = ModeProjectionInputs::default();
        S::register_inputs(&mut inputs);
        for (index, input) in inputs.inputs.iter().enumerate() {
            if inputs.inputs[..index]
                .iter()
                .any(|registered| registered.type_id == input.type_id)
            {
                return Err(ModeProjectionRegistrationError::DuplicateInput {
                    mode: S::NAME,
                    input: input.type_name,
                });
            }
        }
        Ok(Self {
            type_id: TypeId::of::<S>(),
            name: S::NAME,
            inputs: inputs.inputs,
            install: install_mode::<S>,
            publish: publish_mode_projection::<S>,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModeProjectionRegistrationError {
    #[error("mode projection `{mode}` is registered more than once")]
    DuplicateMode { mode: &'static str },
    #[error("mode projection `{mode}` registers input `{input}` more than once")]
    DuplicateInput {
        mode: &'static str,
        input: &'static str,
    },
}

struct ModeProjectionRegistry {
    modes: Vec<ModeProjectionRegistration>,
    subscriptions: Vec<Subscription>,
}

impl ModeProjectionRegistry {
    fn new(
        modes: impl IntoIterator<Item = ModeProjectionRegistration>,
    ) -> Result<Self, ModeProjectionRegistrationError> {
        let mut registered = Vec::<ModeProjectionRegistration>::new();
        for mode in modes {
            if registered
                .iter()
                .any(|current| current.type_id == mode.type_id || current.name == mode.name)
            {
                return Err(ModeProjectionRegistrationError::DuplicateMode { mode: mode.name });
            }
            registered.push(mode);
        }
        Ok(Self {
            modes: registered,
            subscriptions: Vec::new(),
        })
    }

    fn unique_inputs(&self) -> Vec<ModeProjectionInput> {
        let mut inputs = Vec::<ModeProjectionInput>::new();
        for input in self.modes.iter().flat_map(|mode| &mode.inputs) {
            if !inputs
                .iter()
                .any(|registered| registered.type_id == input.type_id)
            {
                inputs.push(*input);
            }
        }
        inputs
    }
}

impl Global for ModeProjectionRegistry {}

pub fn install_mode_projections(cx: &mut App) -> Result<(), ModeProjectionRegistrationError> {
    install_registry(
        cx,
        [
            crate::materials_ui::mode_projection_registration()?,
            crate::scripting_ui::mode_projection_registration()?,
            crate::game_data_catalog::mode_projection_registration()?,
        ],
    )
}

fn install_registry(
    cx: &mut App,
    registrations: impl IntoIterator<Item = ModeProjectionRegistration>,
) -> Result<(), ModeProjectionRegistrationError> {
    let registry = ModeProjectionRegistry::new(registrations)?;
    for mode in &registry.modes {
        (mode.install)(cx);
    }
    let inputs = registry.unique_inputs();
    cx.set_global(registry);
    let subscriptions = inputs
        .into_iter()
        .map(|input| (input.observe)(cx))
        .collect();
    cx.global_mut::<ModeProjectionRegistry>().subscriptions = subscriptions;
    cx.refresh_windows();
    Ok(())
}

fn install_mode<S: ModeProjectionSpec>(cx: &mut App) {
    cx.default_global::<S::State>();
    S::install_actions(cx);
    publish_mode_projection::<S>(cx);
}

pub fn publish_mode_projection<S: ModeProjectionSpec>(cx: &mut App) {
    let projection = S::project(cx.global::<S::State>(), cx);
    cx.set_global(projection);
}

pub fn publish_mode_projection_and_refresh<S: ModeProjectionSpec>(cx: &mut App) {
    publish_mode_projection::<S>(cx);
    cx.refresh_windows();
}

fn observe_input<I: Global>(cx: &mut App) -> Subscription {
    cx.observe_global::<I>(|cx| publish_input(TypeId::of::<I>(), cx))
}

fn publish_input(input: TypeId, cx: &mut App) {
    let publishers = cx
        .global::<ModeProjectionRegistry>()
        .modes
        .iter()
        .filter(|mode| mode.inputs.iter().any(|item| item.type_id == input))
        .map(|mode| mode.publish)
        .collect::<SmallVec<[PublishMode; 4]>>();
    if publishers.is_empty() {
        return;
    }
    for publish in publishers {
        publish(cx);
    }
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestState;

    impl Global for TestState {}

    #[derive(Default)]
    struct PrimaryInput(u32);

    impl Global for PrimaryInput {}

    #[derive(Default)]
    struct OtherInput(u32);

    impl Global for OtherInput {}

    #[derive(Debug, Default, PartialEq, Eq)]
    struct FirstProjection(u32);

    impl Global for FirstProjection {}

    #[derive(Debug, Default, PartialEq, Eq)]
    struct SecondProjection(u32);

    impl Global for SecondProjection {}

    #[derive(Debug, Default, PartialEq, Eq)]
    struct UnrelatedProjection(u32);

    impl Global for UnrelatedProjection {}

    struct FirstMode;

    impl ModeProjectionSpec for FirstMode {
        type State = TestState;
        type Projection = FirstProjection;

        const NAME: &'static str = "first";

        fn register_inputs(inputs: &mut ModeProjectionInputs) {
            inputs.depends_on::<PrimaryInput>();
        }

        fn install_actions(_: &mut App) {}

        fn project(_: &Self::State, cx: &App) -> Self::Projection {
            FirstProjection(cx.try_global::<PrimaryInput>().map_or(0, |input| input.0))
        }
    }

    struct SecondMode;

    impl ModeProjectionSpec for SecondMode {
        type State = TestState;
        type Projection = SecondProjection;

        const NAME: &'static str = "second";

        fn register_inputs(inputs: &mut ModeProjectionInputs) {
            inputs.depends_on::<PrimaryInput>();
        }

        fn install_actions(_: &mut App) {}

        fn project(_: &Self::State, cx: &App) -> Self::Projection {
            SecondProjection(cx.try_global::<PrimaryInput>().map_or(0, |input| input.0))
        }
    }

    struct UnrelatedMode;

    impl ModeProjectionSpec for UnrelatedMode {
        type State = TestState;
        type Projection = UnrelatedProjection;

        const NAME: &'static str = "unrelated";

        fn register_inputs(inputs: &mut ModeProjectionInputs) {
            inputs.depends_on::<OtherInput>();
        }

        fn install_actions(_: &mut App) {}

        fn project(_: &Self::State, cx: &App) -> Self::Projection {
            UnrelatedProjection(cx.try_global::<OtherInput>().map_or(0, |input| input.0))
        }
    }

    struct DuplicateInputMode;

    impl ModeProjectionSpec for DuplicateInputMode {
        type State = TestState;
        type Projection = FirstProjection;

        const NAME: &'static str = "duplicate-input";

        fn register_inputs(inputs: &mut ModeProjectionInputs) {
            inputs.depends_on::<PrimaryInput>();
            inputs.depends_on::<PrimaryInput>();
        }

        fn install_actions(_: &mut App) {}

        fn project(_: &Self::State, _: &App) -> Self::Projection {
            FirstProjection::default()
        }
    }

    #[gpui::test]
    fn input_publication_rebuilds_only_dependent_modes(cx: &gpui::TestAppContext) {
        cx.update(|app| {
            app.set_global(PrimaryInput(7));
            app.set_global(OtherInput(11));
            install_registry(
                app,
                [
                    ModeProjectionRegistration::for_spec::<FirstMode>()
                        .expect("first registration"),
                    ModeProjectionRegistration::for_spec::<SecondMode>()
                        .expect("second registration"),
                    ModeProjectionRegistration::for_spec::<UnrelatedMode>()
                        .expect("unrelated registration"),
                ],
            )
            .expect("install registry");
        });

        assert_eq!(cx.read_global::<FirstProjection, _>(|value, _| value.0), 7);
        assert_eq!(cx.read_global::<SecondProjection, _>(|value, _| value.0), 7);
        assert_eq!(
            cx.read_global::<UnrelatedProjection, _>(|value, _| value.0),
            11
        );

        cx.update(|app| {
            app.set_global(UnrelatedProjection(99));
            app.set_global(PrimaryInput(13));
        });

        assert_eq!(cx.read_global::<FirstProjection, _>(|value, _| value.0), 13);
        assert_eq!(
            cx.read_global::<SecondProjection, _>(|value, _| value.0),
            13
        );
        assert_eq!(
            cx.read_global::<UnrelatedProjection, _>(|value, _| value.0),
            99
        );
    }

    #[gpui::test]
    fn editor_mode_install_publishes_all_initial_projections(cx: &gpui::TestAppContext) {
        cx.update(|app| install_mode_projections(app).expect("install editor modes"));

        cx.read_global::<az_editor_ui::panels::EditorMaterialsProjection, _>(|_, _| ());
        cx.read_global::<az_editor_ui::panels::EditorScriptingProjection, _>(|_, _| ());
        cx.read_global::<az_editor_ui::panels::EditorGameDataProjection, _>(|_, _| ());
    }

    #[gpui::test]
    fn input_published_after_install_in_the_same_update_reaches_dependents(
        cx: &gpui::TestAppContext,
    ) {
        cx.update(|app| {
            install_registry(
                app,
                [
                    ModeProjectionRegistration::for_spec::<FirstMode>()
                        .expect("first registration"),
                ],
            )
            .expect("install registry");
            app.set_global(PrimaryInput(23));
        });

        assert_eq!(cx.read_global::<FirstProjection, _>(|value, _| value.0), 23);
    }

    #[test]
    fn duplicate_mode_registration_is_rejected() {
        let error = ModeProjectionRegistry::new([
            ModeProjectionRegistration::for_spec::<FirstMode>().expect("first registration"),
            ModeProjectionRegistration::for_spec::<FirstMode>().expect("second registration"),
        ])
        .err()
        .expect("duplicate must fail");

        assert_eq!(
            error,
            ModeProjectionRegistrationError::DuplicateMode { mode: "first" }
        );
    }

    #[test]
    fn duplicate_input_registration_is_rejected() {
        let error = ModeProjectionRegistration::for_spec::<DuplicateInputMode>()
            .err()
            .expect("duplicate input must fail");

        assert_eq!(
            error,
            ModeProjectionRegistrationError::DuplicateInput {
                mode: "duplicate-input",
                input: type_name::<PrimaryInput>(),
            }
        );
    }
}
