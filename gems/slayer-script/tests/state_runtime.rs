mod support;

use az_gem_slayer_script::{
    LayerDefinition, LayerId, SequenceId, SequencePhase, SlayerScriptLiteral, StateActionMask,
    StateChanged, StateId, StateRegistrationMetadata, StateTable, StateTableBuilder,
};

use support::{
    LogEntry, TestFunctions, TestModules, TestOperation, runtime_with_states, seconds, sequence,
};

fn state_table(states: Vec<Vec<TestOperation>>) -> StateTable<TestOperation> {
    let mut builder = StateTableBuilder::new();
    for (index, actions) in states.into_iter().enumerate() {
        builder
            .register_state(
                SlayerScriptLiteral {
                    crc: u32::try_from(index + 1).unwrap(),
                },
                StateRegistrationMetadata::PRESERVE_REGISTRATION_ORDER,
                actions,
            )
            .unwrap();
    }
    builder.finalize().unwrap()
}

#[test]
fn layers_start_without_a_state_and_invalid_ids_are_no_ops() {
    let states = state_table(vec![Vec::new()]);
    let mut runtime = runtime_with_states(
        Vec::new(),
        vec![LayerDefinition::new()],
        states,
        TestModules::default(),
        TestFunctions::default(),
    );

    assert_eq!(runtime.current_state(LayerId::new(0)), Some(StateId::NONE));
    runtime
        .switch_state(LayerId::new(0), StateId::NONE, false)
        .unwrap();
    runtime
        .switch_state(LayerId::new(0), StateId::new(9), false)
        .unwrap();

    assert_eq!(runtime.current_state(LayerId::new(0)), Some(StateId::NONE));
    assert!(runtime.functions().log.is_empty());
}

#[test]
fn nested_state_actions_append_to_the_fifo_and_preserve_native_order() {
    let states = state_table(vec![
        vec![
            TestOperation::StateMark("state-0-enter", StateActionMask::ENTER),
            TestOperation::StateSwitch {
                on: StateActionMask::ENTER,
                next: StateId::new(1),
                force: false,
            },
            TestOperation::StateMark("state-0-exit", StateActionMask::EXIT),
        ],
        vec![TestOperation::StateMark(
            "state-1-enter",
            StateActionMask::ENTER,
        )],
    ]);
    let mut runtime = runtime_with_states(
        Vec::new(),
        vec![LayerDefinition::new()],
        states,
        TestModules::default(),
        TestFunctions::default(),
    );

    runtime
        .switch_state(LayerId::new(0), StateId::new(0), false)
        .unwrap();

    assert_eq!(
        runtime.current_state(LayerId::new(0)),
        Some(StateId::new(1))
    );
    assert_eq!(
        runtime.functions().log,
        vec![
            LogEntry::StateMetadataRefreshed(LayerId::new(0)),
            LogEntry::StateOperation("state-0-enter", StateId::new(0), StateActionMask::ENTER),
            LogEntry::StateChanged(StateChanged {
                old_state: StateId::NONE,
                new_state: StateId::new(0),
                current_sequence: None,
            }),
            LogEntry::StateOperation("state-0-exit", StateId::new(0), StateActionMask::EXIT),
            LogEntry::StateMetadataRefreshed(LayerId::new(0)),
            LogEntry::StateOperation("state-1-enter", StateId::new(1), StateActionMask::ENTER),
            LogEntry::StateChanged(StateChanged {
                old_state: StateId::new(0),
                new_state: StateId::new(1),
                current_sequence: None,
            }),
        ]
    );
}

#[test]
fn state_runtime_blocker_and_same_state_suppression_are_non_poisoning() {
    let states = state_table(vec![Vec::new()]);
    let functions = TestFunctions {
        state_blocked: true,
        ..TestFunctions::default()
    };
    let mut runtime = runtime_with_states(
        Vec::new(),
        vec![LayerDefinition::new()],
        states,
        TestModules::default(),
        functions,
    );

    runtime
        .switch_state(LayerId::new(0), StateId::new(0), true)
        .unwrap();
    assert_eq!(runtime.current_state(LayerId::new(0)), Some(StateId::NONE));

    runtime.functions_mut().state_blocked = false;
    runtime
        .switch_state(LayerId::new(0), StateId::new(0), false)
        .unwrap();
    let log_len = runtime.functions().log.len();
    runtime
        .switch_state(LayerId::new(0), StateId::new(0), false)
        .unwrap();
    assert_eq!(runtime.functions().log.len(), log_len);
    runtime
        .switch_state(LayerId::new(0), StateId::new(0), true)
        .unwrap();
    assert!(runtime.functions().log.len() > log_len);
    assert!(!runtime.is_poisoned());
}

#[test]
fn state_action_fifo_silently_drops_requests_beyond_ten_entries() {
    let mut first_actions = Vec::new();
    for state in 1..=11 {
        first_actions.push(TestOperation::StateSwitch {
            on: StateActionMask::ENTER,
            next: StateId::new(state),
            force: false,
        });
    }
    let mut definitions = vec![first_actions];
    definitions.extend((0..11).map(|_| Vec::new()));
    let states = state_table(definitions);
    let mut runtime = runtime_with_states(
        Vec::new(),
        vec![LayerDefinition::new()],
        states,
        TestModules::default(),
        TestFunctions::default(),
    );

    runtime
        .switch_state(LayerId::new(0), StateId::new(0), false)
        .unwrap();

    assert_eq!(
        runtime.current_state(LayerId::new(0)),
        Some(StateId::new(10))
    );
}

#[test]
fn guarded_update_applies_pending_state_before_pending_transition() {
    let states = state_table(vec![Vec::new()]);
    let sequence_zero = sequence(1.0, false).with_actions(vec![
        TestOperation::SwitchState {
            on: SequencePhase::Update,
            next: StateId::new(0),
            force: false,
        },
        TestOperation::Trans {
            on: SequencePhase::Update,
            next: Some(SequenceId::new(1)),
            transition_frames: 0.0,
            initial_time_frames: 0.0,
            force: false,
        },
    ]);
    let mut runtime = runtime_with_states(
        vec![sequence_zero, sequence(1.0, false)],
        vec![LayerDefinition::new()],
        states,
        TestModules::default(),
        TestFunctions::default(),
    );
    runtime
        .trans(
            LayerId::new(0),
            az_gem_slayer_script::TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.functions_mut().log.clear();

    runtime.update(seconds(0.1)).unwrap();

    let state_position = runtime
        .functions()
        .log
        .iter()
        .position(|entry| matches!(entry, LogEntry::StateChanged(_)))
        .unwrap();
    let sequence_position = runtime
        .functions()
        .log
        .iter()
        .position(|entry| matches!(entry, LogEntry::Changed(_)))
        .unwrap();
    assert!(state_position < sequence_position);
    assert!(runtime.functions().log.iter().any(|entry| matches!(
        entry,
        LogEntry::StateChanged(StateChanged {
            current_sequence: Some(sequence),
            ..
        }) if *sequence == SequenceId::new(0)
    )));
}

#[test]
fn first_update_on_start_can_install_initial_state_synchronously() {
    let states = state_table(vec![Vec::new()]);
    let modules = TestModules {
        runtime_state_switch: Some(StateId::new(0)),
        ..TestModules::default()
    };
    let mut runtime = runtime_with_states(
        Vec::new(),
        vec![LayerDefinition::new()],
        states,
        modules,
        TestFunctions::default(),
    );

    runtime.update(seconds(0.0)).unwrap();

    assert_eq!(
        runtime.current_state(LayerId::new(0)),
        Some(StateId::new(0))
    );
}
