mod support;

use az_gem_slayer_script::{
    AuthoredFrames, EventCallbackPhase, LayerDefinition, LayerId, LayerKind, LayerPlaybackRate,
    ParentSequenceChanged, ParentSequenceContext, ResolvedParentId, SequenceActionMask, SequenceId,
    SequencePhase, TransitionOutcome, TransitionRequest,
};

use support::{LogEntry, TestOperation, frames, runtime, runtime_with, seconds, sequence};

fn install(runtime: &mut support::TestRuntime, sequence: u32) {
    assert!(matches!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(sequence))),
            )
            .unwrap(),
        TransitionOutcome::Applied(_)
    ));
}

#[test]
fn layers_start_empty_and_normal_transition_orders_exit_change_enter() {
    let old = sequence(2.0, false).with_actions(vec![
        TestOperation::Mark("old-enter", SequencePhase::Enter),
        TestOperation::Mark("old-exit", SequencePhase::Exit),
    ]);
    let new = sequence(2.0, false)
        .with_actions(vec![TestOperation::Mark("new-enter", SequencePhase::Enter)]);
    let mut runtime = runtime(vec![old, new], vec![LayerDefinition::new()]);
    assert_eq!(runtime.layer(LayerId::new(0)).unwrap().current(), None);

    install(&mut runtime, 0);
    runtime.functions_mut().log.clear();
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(1))),
        )
        .unwrap();

    assert_eq!(
        runtime.functions().log,
        vec![
            LogEntry::Operation("old-exit", SequencePhase::Exit, SequenceActionMask::EXIT),
            LogEntry::Changed(az_gem_slayer_script::SequenceChanged {
                layer: LayerId::new(0),
                previous: Some(SequenceId::new(0)),
                current: Some(SequenceId::new(1)),
            }),
            LogEntry::Operation("new-enter", SequencePhase::Enter, SequenceActionMask::ENTER),
        ]
    );
}

#[test]
fn force_transition_skips_old_cleanup_actions_and_new_enter_actions() {
    let old =
        sequence(1.0, false).with_actions(vec![TestOperation::Mark("exit", SequencePhase::Exit)]);
    let new =
        sequence(1.0, false).with_actions(vec![TestOperation::Mark("enter", SequencePhase::Enter)]);
    let mut runtime = runtime(vec![old, new], vec![LayerDefinition::new()]);
    install(&mut runtime, 0);
    runtime.functions_mut().log.clear();

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::new(
                Some(SequenceId::new(1)),
                AuthoredFrames::ZERO,
                AuthoredFrames::ZERO,
                true,
            ),
        )
        .unwrap();

    assert_eq!(runtime.functions().log.len(), 1);
    assert!(matches!(runtime.functions().log[0], LogEntry::Changed(_)));
}

#[test]
fn guarded_update_transition_uses_the_single_pending_slot() {
    let request_one = TestOperation::Trans {
        on: SequencePhase::Update,
        next: Some(SequenceId::new(1)),
        transition_frames: 0.0,
        initial_time_frames: 0.0,
        force: false,
    };
    let request_two = TestOperation::Trans {
        on: SequencePhase::Update,
        next: Some(SequenceId::new(2)),
        transition_frames: 0.0,
        initial_time_frames: 0.0,
        force: false,
    };
    let mut runtime = runtime(
        vec![
            sequence(1.0, false).with_actions(vec![request_one, request_two]),
            sequence(1.0, false),
            sequence(1.0, false),
        ],
        vec![LayerDefinition::new()],
    );
    install(&mut runtime, 0);

    runtime.update(seconds(0.1)).unwrap();

    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(2)),
        "the last guarded normal non-null request wins before pending application"
    );
}

#[test]
fn guarded_transition_preflights_at_request_and_again_at_pending_apply() {
    let request = TestOperation::Trans {
        on: SequencePhase::Update,
        next: Some(SequenceId::new(1)),
        transition_frames: 0.0,
        initial_time_frames: 0.0,
        force: false,
    };
    let mut runtime = runtime(
        vec![
            sequence(1.0, false).with_actions(vec![request]),
            sequence(1.0, false),
        ],
        vec![LayerDefinition::new()],
    );
    install(&mut runtime, 0);
    runtime.functions_mut().transition_preflight_log.clear();

    runtime.update(seconds(0.1)).unwrap();

    assert_eq!(
        runtime.functions().transition_preflight_log,
        vec!["target", "lifecycle", "target", "lifecycle"]
    );
}

#[test]
fn target_guard_runs_before_lifecycle_and_short_circuits_it() {
    let functions = support::TestFunctions {
        reject_target: Some(SequenceId::new(0)),
        lifecycle_blocked: true,
        ..support::TestFunctions::default()
    };
    let mut runtime = runtime_with(
        vec![sequence(1.0, false)],
        vec![LayerDefinition::new()],
        support::TestModules::default(),
        functions,
    );

    assert_eq!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap(),
        TransitionOutcome::BlockedByTarget {
            sequence: SequenceId::new(0)
        }
    );
    assert_eq!(runtime.functions().transition_preflight_log, vec!["target"]);
}

#[test]
fn lifecycle_target_and_transition_count_blocks_are_non_poisoning_outcomes() {
    let functions = support::TestFunctions {
        lifecycle_blocked: true,
        ..support::TestFunctions::default()
    };
    let mut runtime = runtime_with(
        vec![sequence(1.0, false)],
        vec![LayerDefinition::new()],
        support::TestModules::default(),
        functions,
    );
    assert_eq!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap(),
        TransitionOutcome::BlockedByLifecycle
    );
    assert_eq!(runtime.layer(LayerId::new(0)).unwrap().current(), None);
    assert!(!runtime.is_poisoned());

    runtime.functions_mut().lifecycle_blocked = false;
    runtime.functions_mut().reject_target = Some(SequenceId::new(0));
    assert_eq!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap(),
        TransitionOutcome::BlockedByTarget {
            sequence: SequenceId::new(0)
        }
    );
    runtime.functions_mut().reject_target = None;
    for _ in 0..10 {
        assert!(matches!(
            runtime
                .trans(
                    LayerId::new(0),
                    TransitionRequest::immediate(Some(SequenceId::new(0))),
                )
                .unwrap(),
            TransitionOutcome::Applied(_)
        ));
    }
    assert_eq!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap(),
        TransitionOutcome::IgnoredNestingLimit
    );
    runtime.update(seconds(0.1)).unwrap();
    assert!(matches!(
        runtime
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(0))),
            )
            .unwrap(),
        TransitionOutcome::Applied(_)
    ));
}

// The clamp lands on the authored duration exactly; an epsilon would not
// distinguish clamped from nearly-clamped.
#[allow(clippy::float_cmp)]
#[test]
fn current_layer_clamps_at_duration_for_loop_and_end() {
    for looping in [false, true] {
        let mut runtime = runtime(vec![sequence(1.0, looping)], vec![LayerDefinition::new()]);
        install(&mut runtime, 0);
        runtime.update(seconds(1.5)).unwrap();
        let layer = runtime.layer(LayerId::new(0)).unwrap();
        assert_eq!(layer.current_time_seconds(), 1.0);
        assert_eq!(layer.wrapped_this_step(), looping);
        assert_eq!(layer.reached_end(), !looping);
    }
}

#[test]
fn one_action_object_receives_the_combined_update_and_end_mask() {
    let sequence = sequence(1.0, false).with_actions(vec![TestOperation::RecordMask("combined")]);
    let mut runtime = runtime(vec![sequence], vec![LayerDefinition::new()]);
    install(&mut runtime, 0);
    runtime.functions_mut().log.clear();

    runtime.update(seconds(1.5)).unwrap();

    assert!(runtime.functions().log.iter().any(|entry| matches!(
        entry,
        LogEntry::Operation("combined", SequencePhase::Update, mask)
            if mask.bits() == 0x8018
    )));
}

#[test]
fn parent_actions_run_deepest_first_and_scope_callback_runtime_ids() {
    let parent = sequence(1.0, false).with_actions(vec![
        TestOperation::Mark("parent", SequencePhase::Update),
        TestOperation::RegisterCallback {
            on: SequencePhase::Update,
            authored_id: 7,
            label: "parent-callback",
            start_seconds: 0.0,
            end_seconds: 1.0,
            may_defer: false,
        },
    ]);
    let child = sequence(1.0, false)
        .with_parent_sequence(SequenceId::new(0))
        .with_actions(vec![
            TestOperation::Mark("child", SequencePhase::Update),
            TestOperation::RegisterCallback {
                on: SequencePhase::Update,
                authored_id: 7,
                label: "child-callback",
                start_seconds: 0.0,
                end_seconds: 1.0,
                may_defer: false,
            },
        ]);
    let mut runtime = runtime(vec![parent, child], vec![LayerDefinition::new()]);
    install(&mut runtime, 1);
    runtime.functions_mut().log.clear();

    runtime.update(seconds(0.25)).unwrap();

    let operation_labels = runtime
        .functions()
        .log
        .iter()
        .filter_map(|entry| match entry {
            LogEntry::Operation(label, SequencePhase::Update, _) => Some(*label),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(operation_labels, vec!["parent", "child"]);
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(id, phase, delta)| (id.get(), *phase, *delta))
            .collect::<Vec<_>>(),
        vec![
            (7, EventCallbackPhase::Enter, 0.25),
            (7, EventCallbackPhase::Update, 0.25),
            (1_000_007, EventCallbackPhase::Enter, 0.25),
            (1_000_007, EventCallbackPhase::Update, 0.25),
        ]
    );
}

#[test]
fn dynamically_registered_callback_keeps_phase_order_during_immediate_clear() {
    let sequence =
        sequence(1.0, false).with_actions(vec![TestOperation::RegisterTransitionCallback {
            on: SequencePhase::Update,
            authored_id: 23,
            label: "registered-reentrant",
            start_seconds: 0.0,
            end_seconds: 1.0,
            transition_on: EventCallbackPhase::Enter,
            next: None,
        }]);
    let mut runtime = runtime(vec![sequence], vec![LayerDefinition::new()]);
    install(&mut runtime, 0);
    runtime.functions_mut().current_event_callbacks.clear();

    runtime.update(seconds(0.25)).unwrap();

    assert_eq!(runtime.layer(LayerId::new(0)).unwrap().current(), None);
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(id, phase, _)| (id.get(), *phase))
            .collect::<Vec<_>>(),
        vec![
            (23, EventCallbackPhase::Enter),
            (23, EventCallbackPhase::Update),
        ]
    );
}

#[test]
fn auxiliary_pending_transition_stops_the_callback_walk_and_remains_stranded() {
    let auxiliary = sequence(1.0, false).with_actions(vec![
        TestOperation::RegisterCallback {
            on: SequencePhase::Update,
            authored_id: 13,
            label: "auxiliary",
            start_seconds: 0.0,
            end_seconds: 1.0,
            may_defer: false,
        },
        TestOperation::Trans {
            on: SequencePhase::Update,
            next: Some(SequenceId::new(1)),
            transition_frames: 0.0,
            initial_time_frames: 0.0,
            force: false,
        },
    ]);
    let target = sequence(1.0, false);
    let mut runtime = runtime(
        vec![auxiliary, target],
        vec![LayerDefinition::new().with_kind(LayerKind::Auxiliary)],
    );
    install(&mut runtime, 0);

    runtime.update(seconds(0.25)).unwrap();

    assert!(runtime.functions().current_event_callbacks.is_empty());
    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(0))
    );

    runtime.update(seconds(0.25)).unwrap();
    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(0)),
        "native auxiliary pending transition storage is not drained by +0x608d3e0"
    );
}

#[test]
fn immediate_auxiliary_clear_stops_the_redirected_record_root() {
    let auxiliary = sequence(1.0, false).with_actions(vec![
        TestOperation::RegisterCallback {
            on: SequencePhase::Update,
            authored_id: 19,
            label: "must-be-stopped-before-aux-dispatch",
            start_seconds: 0.0,
            end_seconds: 1.0,
            may_defer: false,
        },
        TestOperation::Trans {
            on: SequencePhase::Update,
            next: None,
            transition_frames: 0.0,
            initial_time_frames: 0.0,
            force: false,
        },
    ]);
    let mut runtime = runtime(
        vec![auxiliary],
        vec![LayerDefinition::new().with_kind(LayerKind::Auxiliary)],
    );
    install(&mut runtime, 0);

    runtime.update(seconds(0.25)).unwrap();

    assert_eq!(runtime.layer(LayerId::new(0)).unwrap().current(), None);
    assert!(runtime.functions().current_event_callbacks.is_empty());
}

#[test]
fn auxiliary_deferred_queue_uses_the_restored_layer_root_lookup_and_drops_a_miss() {
    let auxiliary = sequence(1.0, false).with_actions(vec![TestOperation::RegisterCallback {
        on: SequencePhase::Update,
        authored_id: 17,
        label: "auxiliary-deferred",
        start_seconds: 0.0,
        end_seconds: 1.0,
        may_defer: true,
    }]);
    let mut runtime = runtime(
        vec![auxiliary],
        vec![LayerDefinition::new().with_kind(LayerKind::Auxiliary)],
    );
    install(&mut runtime, 0);

    runtime.update(seconds(0.25)).unwrap();
    runtime.flush_interval_callbacks().unwrap();

    assert!(runtime.functions().current_event_callbacks.is_empty());
}

// The blend record must be bit-identical to `raw_before`, and the clock
// lands on an exactly representable 0.25.
#[allow(clippy::float_cmp)]
#[test]
fn negative_layer_rate_moves_current_clock_backward_but_not_blend_records() {
    let layer = LayerDefinition::new().with_playback_rate(LayerPlaybackRate::new(-1.0).unwrap());
    let mut runtime = runtime(vec![sequence(2.0, false)], vec![layer]);
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::new(Some(SequenceId::new(0)), frames(30.0), frames(15.0), false),
        )
        .unwrap();
    let raw_before = runtime.layer(LayerId::new(0)).unwrap().records()[0].raw_transition_progress();

    runtime.update(seconds(0.25)).unwrap();

    let layer = runtime.layer(LayerId::new(0)).unwrap();
    assert_eq!(layer.current_time_seconds(), 0.25);
    assert_eq!(layer.records()[0].raw_transition_progress(), raw_before);
}

#[test]
fn explicit_clear_has_no_fabricated_completion_event() {
    let mut runtime = runtime(vec![sequence(1.0, false)], vec![LayerDefinition::new()]);
    install(&mut runtime, 0);
    let outcome = runtime
        .trans(LayerId::new(0), TransitionRequest::immediate(None))
        .unwrap();

    assert!(matches!(outcome, TransitionOutcome::Applied(_)));
    assert_eq!(runtime.layer(LayerId::new(0)).unwrap().current(), None);
}

#[test]
fn unguarded_adapter_transition_applies_synchronously() {
    let modules = support::TestModules {
        dispatch_transition: Some(TransitionRequest::immediate(Some(SequenceId::new(0)))),
        ..support::TestModules::default()
    };
    let mut runtime = runtime_with(
        vec![sequence(1.0, false)],
        vec![LayerDefinition::new()],
        modules,
        support::TestFunctions::default(),
    );

    runtime
        .dispatch_typed(&support::TypedEvent {
            owner: az_gem_slayer_script::ModuleId::new(7),
            value: 1,
        })
        .unwrap();
    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(0))
    );
}

#[test]
fn applied_transition_fanouts_exact_parent_sequence_payload() {
    let context = ParentSequenceContext {
        parent: ResolvedParentId::new(17),
        resolved_value_words: [1, 2, 3, 4, 5, 6],
        state_words: [7, 8, 9, 10, 11],
    };
    let functions = support::TestFunctions {
        capture_parent_events: true,
        ..support::TestFunctions::default()
    };
    let mut runtime = runtime_with(
        vec![sequence(1.0, false).with_parent_context(context)],
        vec![LayerDefinition::new()],
        support::TestModules::default(),
        functions,
    );

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::new(Some(SequenceId::new(0)), frames(6.0), frames(-3.0), false),
        )
        .unwrap();

    assert!(
        runtime
            .functions()
            .log
            .contains(&LogEntry::ParentChanged(ParentSequenceChanged {
                parent: ResolvedParentId::new(17),
                resolved_value_words: [1, 2, 3, 4, 5, 6],
                transition_frames: 6.0,
                initial_time_frames: -3.0,
                state_words: [7, 8, 9, 10, 11],
                layer: LayerId::new(0),
            }))
    );
}
