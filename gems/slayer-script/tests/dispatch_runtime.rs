mod support;

use az_gem_slayer_script::{
    CustomDispatchTarget, LayerDefinition, LayerId, ModuleId, SequenceId, TransitionRequest,
};

use support::{
    CustomEvent, LogEntry, TestModules, TypedEvent, fanout, runtime, runtime_with, sequence,
};

#[test]
fn typed_targeted_and_fanout_dispatch_are_direct_and_stable() {
    let mut runtime = runtime(Vec::new(), Vec::new());

    runtime
        .dispatch_typed(&TypedEvent {
            owner: ModuleId::new(9),
            value: 1,
        })
        .unwrap();
    runtime
        .dispatch_custom(
            CustomDispatchTarget::Module(ModuleId::new(2)),
            &CustomEvent(2),
        )
        .unwrap();
    runtime.dispatch_custom(fanout(), &CustomEvent(4)).unwrap();

    assert_eq!(
        runtime.functions().log,
        vec![
            LogEntry::Typed(ModuleId::new(9), 1),
            LogEntry::Custom(ModuleId::new(2), 2),
            LogEntry::Custom(ModuleId::new(2), 4),
            LogEntry::Custom(ModuleId::new(5), 4),
        ]
    );
}

#[test]
fn direct_typed_dispatch_applies_synchronous_reentry_at_the_call_site() {
    let modules = TestModules {
        dispatch_transition: Some(TransitionRequest::immediate(Some(SequenceId::new(1)))),
        ..TestModules::default()
    };
    let mut runtime = runtime_with(
        vec![sequence(1.0, false), sequence(1.0, false)],
        vec![LayerDefinition::new()],
        modules,
        support::TestFunctions::default(),
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    runtime
        .dispatch_typed(&TypedEvent {
            owner: ModuleId::new(9),
            value: 1,
        })
        .unwrap();
    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(1))
    );
    assert!(!runtime.is_poisoned());
}

#[test]
fn targeted_and_fanout_dispatch_apply_synchronous_reentry() {
    for target in [
        CustomDispatchTarget::Module(ModuleId::new(2)),
        CustomDispatchTarget::Fanout,
    ] {
        let modules = TestModules {
            dispatch_transition: Some(TransitionRequest::immediate(Some(SequenceId::new(1)))),
            ..TestModules::default()
        };
        let mut runtime = runtime_with(
            vec![sequence(1.0, false), sequence(1.0, false)],
            vec![LayerDefinition::new()],
            modules,
            support::TestFunctions::default(),
        );
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap();

        runtime.dispatch_custom(target, &CustomEvent(3)).unwrap();

        assert_eq!(
            runtime.layer(LayerId::new(0)).unwrap().current(),
            Some(SequenceId::new(1))
        );
    }
}

#[test]
fn unknown_target_zero_fanout_and_missing_typed_owner_are_no_ops() {
    let mut unknown = runtime(Vec::new(), Vec::new());
    unknown
        .dispatch_custom(
            CustomDispatchTarget::Module(ModuleId::new(99)),
            &CustomEvent(1),
        )
        .unwrap();
    assert!(!unknown.is_poisoned());

    let mut empty = runtime_with(
        Vec::new(),
        Vec::new(),
        TestModules {
            custom_acceptors: Vec::new(),
            ..TestModules::default()
        },
        support::TestFunctions::default(),
    );
    empty.dispatch_custom(fanout(), &CustomEvent(2)).unwrap();

    let mut missing = runtime(Vec::new(), Vec::new());
    missing
        .dispatch_typed(&TypedEvent {
            owner: ModuleId::new(1),
            value: u8::MAX,
        })
        .unwrap();
}

#[test]
fn mutable_function_adapter_access_does_not_expose_runtime_state() {
    let mut runtime = runtime(Vec::new(), Vec::new());
    runtime.functions_mut().external_increment = Some(0.75);

    assert_eq!(runtime.functions().external_increment, Some(0.75));
    assert!(runtime.layer(LayerId::new(0)).is_none());
}

#[test]
fn on_start_is_synchronous_once_before_remaining_update_work() {
    let mut runtime = runtime(Vec::new(), Vec::new());

    runtime.update(support::seconds(0.0)).unwrap();
    runtime.update(support::seconds(0.25)).unwrap();

    assert_eq!(
        runtime.functions().log,
        vec![
            LogEntry::OnStart(ModuleId::new(7)),
            LogEntry::ModuleUpdate,
            LogEntry::ModuleUpdate,
        ]
    );
}
