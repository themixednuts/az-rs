mod support;

use az_gem_slayer_script::{
    AuthoredEventGroupCount, BoundEventProperties, CallbackAuthoredId, CurrentEventHostExecution,
    EventCallbackPhase, EventIntervalDefinition, EventRootDefinition, EventTrackDefinition,
    ExecutableEventChannelCount, ExecutableEventId, IntervalCallbackDefinition, LayerDefinition,
    LayerId, ModuleId, PayloadEventTrackDefinition, SequenceDefinition, SequenceId, StateTable,
    TransitionRequest,
};

use support::{
    TestCallback, TestFunctions, TestModules, runtime, runtime_with, runtime_with_states_and_mode,
    seconds, sequence,
};

fn interval(
    event_id: u32,
    external: bool,
    boundary: f32,
    offset: f32,
    callbacks: Vec<IntervalCallbackDefinition<TestCallback>>,
) -> EventIntervalDefinition<TestCallback> {
    EventIntervalDefinition::new(
        seconds(0.0),
        seconds(1.0),
        EventRootDefinition::new(ExecutableEventId::new(event_id), seconds(1.0), external).unwrap(),
        BoundEventProperties::new(1.0, boundary, offset, 0.2, 0.75, false).unwrap(),
        callbacks,
    )
    .unwrap()
}

fn event_layer() -> LayerDefinition {
    event_layer_with_count(1)
}

fn event_layer_with_count(count: i32) -> LayerDefinition {
    LayerDefinition::new()
        .with_executable_event_channel_count(ExecutableEventChannelCount::new(count).unwrap())
}

#[test]
fn embedded_transition_tracks_do_not_materialize_current_executable_slots() {
    let track = EventTrackDefinition::new(vec![interval(31, false, 0.0, 0.0, Vec::new())]).unwrap();
    let mut runtime = runtime(
        vec![sequence(1.0, false).with_embedded_event_tracks(vec![track])],
        vec![LayerDefinition::new()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();

    let layer = runtime.layer(LayerId::new(0)).unwrap();
    assert!(layer.current_primary_event_tracks().is_empty());
    assert_eq!(layer.records()[0].embedded_primary_event_tracks().len(), 1);
    assert!(runtime.functions().current_event_starts.is_empty());
}

#[test]
fn transition_stops_old_authored_primary_channels_missing_from_new_slots() {
    let first = EventTrackDefinition::new(vec![interval(41, false, 0.0, 0.0, Vec::new())]).unwrap();
    let second =
        EventTrackDefinition::new(vec![interval(42, false, 0.0, 0.0, Vec::new())]).unwrap();
    let old = sequence(1.0, false)
        .with_authored_primary_event_group_count(AuthoredEventGroupCount::new(2))
        .with_executable_event_tracks(vec![first.clone(), second]);
    let new = sequence(1.0, false)
        .with_authored_primary_event_group_count(AuthoredEventGroupCount::new(1))
        .with_executable_event_tracks(vec![first]);
    let mut runtime = runtime(vec![old, new], vec![event_layer_with_count(2)]);
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(1))),
        )
        .unwrap();

    assert!(
        runtime
            .functions()
            .current_event_stops
            .iter()
            .any(|request| {
                request.channel.get() == 1 && request.fade_seconds.to_bits() == 0.0_f32.to_bits()
            })
    );
    assert_eq!(
        runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_primary_event_tracks()
            .len(),
        1
    );
}

#[test]
fn authored_payload_without_compiled_slot_stops_its_resolved_owner_channel() {
    let primary =
        EventTrackDefinition::new(vec![interval(51, false, 0.0, 0.0, Vec::new())]).unwrap();
    let definition = with_executable_tracks(sequence(1.0, false), vec![primary])
        .with_executable_payload_event_tracks(vec![
            PayloadEventTrackDefinition::without_executable_slot(ModuleId::new(11)),
        ]);
    let mut runtime = runtime(vec![definition], vec![event_layer()]);
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    assert_eq!(runtime.modules().payload_event_stops.len(), 1);
    assert_eq!(runtime.modules().payload_event_stops[0].channel.get(), 0);
    assert!(
        runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_payload_event_tracks()[0]
            .is_none()
    );
}

fn with_executable_tracks(
    definition: SequenceDefinition<support::TestOperation, TestCallback>,
    tracks: Vec<EventTrackDefinition<TestCallback>>,
) -> SequenceDefinition<support::TestOperation, TestCallback> {
    definition
        .with_authored_primary_event_group_count(AuthoredEventGroupCount::new(
            u32::try_from(tracks.len()).unwrap(),
        ))
        .with_executable_event_tracks(tracks)
}

// The start request carries the exact literals the fixture authored; an
// epsilon would stop pinning them.
#[allow(clippy::float_cmp)]
#[test]
fn transition_materializes_fixed_slots_without_dispatching_callbacks() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(1),
        seconds(0.1),
        seconds(0.8),
        TestCallback::direct("primary"),
    )
    .with_deferred_dispatch(true);
    let track =
        EventTrackDefinition::new(vec![interval(17, false, 0.0, 0.25, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    assert!(runtime.functions().current_event_callbacks.is_empty());
    assert_eq!(runtime.functions().current_event_starts.len(), 1);
    let start = runtime.functions().current_event_starts[0];
    assert_eq!(start.channel.get(), 0);
    assert_eq!(start.event_id, ExecutableEventId::new(17));
    assert_eq!(start.fixed_weight, 1.0);
    assert_eq!(start.normalized_start, 0.25);
    assert_eq!(start.fade_seconds, 0.0);
    assert_eq!(start.authored_weight, 0.75);
    assert_eq!(
        runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_primary_event_tracks()
            .len(),
        1
    );
}

#[test]
fn retained_deferrable_self_transition_flushes_exit_before_queued_update() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(2),
        seconds(0.1),
        seconds(0.8),
        TestCallback::direct("retained"),
    )
    .with_deferred_dispatch(true);
    let track =
        EventTrackDefinition::new(vec![interval(91, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();
    runtime.functions_mut().current_event_callbacks.clear();

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    assert!(runtime.functions().current_event_callbacks.is_empty());
    runtime.flush_interval_callbacks().unwrap();

    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Exit, EventCallbackPhase::Update]
    );
}

#[test]
fn queued_exit_transition_observes_and_stops_the_inflight_retained_node() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(12),
        seconds(0.1),
        seconds(0.8),
        TestCallback {
            label: "queued-reentrant",
            transition: Some(SequenceId::new(1)),
            transition_on: Some(EventCallbackPhase::Exit),
        },
    )
    .with_deferred_dispatch(true);
    let track =
        EventTrackDefinition::new(vec![interval(96, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![
            with_executable_tracks(sequence(1.0, false), vec![track]),
            sequence(1.0, false),
        ],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();
    runtime.functions_mut().current_event_callbacks.clear();

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.flush_interval_callbacks().unwrap();

    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(1))
    );
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Exit],
        "the stopped in-flight node must not be resurrected for its queued UPDATE"
    );
}

#[test]
fn different_sequence_retained_callback_is_stopped_without_a_fabricated_queue_exit() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(3),
        seconds(0.1),
        seconds(0.8),
        TestCallback::direct("retained"),
    )
    .with_deferred_dispatch(true);
    let track =
        EventTrackDefinition::new(vec![interval(92, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![
            with_executable_tracks(sequence(1.0, false), vec![track]),
            sequence(1.0, false),
        ],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();
    runtime.functions_mut().current_event_callbacks.clear();

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(1))),
        )
        .unwrap();
    runtime.flush_interval_callbacks().unwrap();

    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Exit]
    );
    assert_eq!(
        runtime.functions().callback_lifecycle,
        vec![
            ("bind", "retained"),
            ("initialize", "retained"),
            ("finalize", "retained"),
        ]
    );
}

#[test]
fn opaque_host_suppression_keeps_gate_and_callback_execution_live() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(4),
        seconds(0.0),
        seconds(0.8),
        TestCallback::direct("suppressed"),
    );
    let track =
        EventTrackDefinition::new(vec![interval(93, true, 0.5, 0.0, vec![callback])]).unwrap();
    let functions = TestFunctions {
        current_event_gate_open: true,
        ..TestFunctions::default()
    };
    let mut runtime = runtime_with_states_and_mode(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
        StateTable::empty(),
        TestModules::default(),
        functions,
        CurrentEventHostExecution::Suppressed,
    );

    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();

    let functions = runtime.functions();
    assert!(functions.current_event_starts.is_empty());
    assert!(functions.current_event_stops.is_empty());
    assert!(functions.current_event_updates.is_empty());
    assert!(functions.current_event_steps.is_empty());
    assert_eq!(
        functions.current_event_gates,
        vec![ExecutableEventId::new(93)]
    );
    assert_eq!(
        functions
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Enter, EventCallbackPhase::Update]
    );
}

#[test]
fn current_callbacks_are_direct_and_phase_state_changes_after_each_call() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(5),
        seconds(0.1),
        seconds(0.8),
        TestCallback::direct("primary"),
    )
    .with_deferred_dispatch(true);
    let track =
        EventTrackDefinition::new(vec![interval(1, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    runtime.update(seconds(0.25)).unwrap();
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Enter, EventCallbackPhase::Update]
    );

    runtime.update(seconds(0.75)).unwrap();
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .last()
            .unwrap()
            .1,
        EventCallbackPhase::Exit
    );
}

#[test]
fn duplicate_wrapping_callback_id_keeps_the_first_registered_object() {
    let first = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(41),
        seconds(0.0),
        seconds(0.8),
        TestCallback::direct("first"),
    );
    let duplicate = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(41),
        seconds(0.0),
        seconds(0.8),
        TestCallback::direct("duplicate"),
    );
    let track =
        EventTrackDefinition::new(vec![interval(96, false, 0.0, 0.0, vec![first, duplicate])])
            .unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    runtime.update(seconds(0.25)).unwrap();

    let labels = runtime
        .functions()
        .log
        .iter()
        .filter_map(|entry| match entry {
            support::LogEntry::Callback(label, _, _) => Some(*label),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["first", "first"]);
    assert_eq!(
        runtime.functions().callback_lifecycle,
        vec![
            ("bind", "first"),
            ("initialize", "first"),
            ("bind", "duplicate"),
            ("initialize", "duplicate"),
        ],
        "native binds and initializes an equal-ID callback before rejecting insertion"
    );
}

#[test]
fn direct_callback_transition_applies_between_phases_without_overwriting_new_slots() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(6),
        seconds(0.0),
        seconds(0.8),
        TestCallback {
            label: "reentrant",
            transition: Some(SequenceId::new(1)),
            transition_on: Some(EventCallbackPhase::Enter),
        },
    );
    let later_callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(7),
        seconds(0.0),
        seconds(0.8),
        TestCallback::direct("later-old-tree-node"),
    );
    let track = EventTrackDefinition::new(vec![interval(
        94,
        false,
        0.0,
        0.0,
        vec![callback, later_callback],
    )])
    .unwrap();
    let mut runtime = runtime(
        vec![
            with_executable_tracks(sequence(1.0, false), vec![track]),
            sequence(1.0, false),
        ],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();

    runtime.update(seconds(0.25)).unwrap();

    assert_eq!(
        runtime.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(1))
    );
    assert!(
        runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_primary_event_tracks()
            .is_empty(),
        "detached old callback traversal must not overwrite the new sequence's slots"
    );
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Enter, EventCallbackPhase::Update]
    );
    assert!(
        runtime.functions().log.iter().all(|entry| !matches!(
            entry,
            support::LogEntry::Callback("later-old-tree-node", _, _)
        )),
        "synchronous Trans must stop the remaining callbacks in the resident old slot"
    );
}

#[test]
fn synchronous_stop_exit_transition_hits_the_typed_safe_rust_boundary() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(7),
        seconds(0.0),
        seconds(0.8),
        TestCallback {
            label: "stop-exit",
            transition: Some(SequenceId::new(1)),
            transition_on: Some(EventCallbackPhase::Exit),
        },
    );
    let track =
        EventTrackDefinition::new(vec![interval(95, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![
            with_executable_tracks(sequence(1.0, false), vec![track]),
            sequence(1.0, false),
        ],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();

    assert!(matches!(
        runtime.trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(1))),
        ),
        Err(az_gem_slayer_script::RuntimeError::UnsafeStopExitReentry {
            layer
        }) if layer == LayerId::new(0)
    ));
    assert!(runtime.is_poisoned());
}

#[test]
fn payload_host_calls_use_compiled_owner_but_callback_object_dispatch_is_direct() {
    let callback = IntervalCallbackDefinition::new(
        CallbackAuthoredId::new(8),
        seconds(0.0),
        seconds(0.9),
        TestCallback::direct("payload"),
    );
    let primary =
        EventTrackDefinition::new(vec![interval(2, false, 0.0, 0.0, Vec::new())]).unwrap();
    let payload =
        EventTrackDefinition::new(vec![interval(3, false, 0.0, 0.0, vec![callback])]).unwrap();
    let mut runtime = runtime(
        vec![
            with_executable_tracks(sequence(1.0, false), vec![primary])
                .with_executable_payload_event_tracks(vec![PayloadEventTrackDefinition::new(
                    ModuleId::new(11),
                    payload,
                )]),
        ],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();

    assert_eq!(runtime.modules().payload_event_starts.len(), 1);
    assert_eq!(runtime.modules().payload_event_updates.len(), 1);
    assert_eq!(
        runtime
            .functions()
            .current_event_callbacks
            .iter()
            .map(|(_, phase, _)| *phase)
            .collect::<Vec<_>>(),
        vec![EventCallbackPhase::Enter, EventCallbackPhase::Update]
    );
}

// `after == before` is the whole assertion: the frozen slot must be
// bit-identical, which an epsilon would not pin.
#[allow(clippy::float_cmp)]
#[test]
fn unresolved_payload_owner_omits_initial_slot_and_freezes_existing_slot() {
    let primary =
        EventTrackDefinition::new(vec![interval(2, false, 0.0, 0.0, Vec::new())]).unwrap();
    let payload =
        EventTrackDefinition::new(vec![interval(3, false, 0.0, 0.0, Vec::new())]).unwrap();
    let definition = with_executable_tracks(sequence(1.0, false), vec![primary.clone()])
        .with_executable_payload_event_tracks(vec![PayloadEventTrackDefinition::new(
            ModuleId::new(11),
            payload.clone(),
        )]);
    let mut missing = TestModules::default();
    missing.payload_event_owners.clear();
    let mut missing_runtime = runtime_with(
        vec![definition],
        vec![event_layer()],
        missing,
        TestFunctions::default(),
    );
    missing_runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    assert!(
        missing_runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_payload_event_tracks()[0]
            .is_none()
    );

    let definition = with_executable_tracks(sequence(1.0, false), vec![primary])
        .with_executable_payload_event_tracks(vec![PayloadEventTrackDefinition::new(
            ModuleId::new(11),
            payload,
        )]);
    let mut runtime = runtime(vec![definition], vec![event_layer()]);
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.update(seconds(0.25)).unwrap();
    let before = runtime
        .layer(LayerId::new(0))
        .unwrap()
        .current_payload_event_tracks()[0]
        .as_ref()
        .unwrap()
        .playback_seconds();
    let update_count = runtime.modules().payload_event_updates.len();

    runtime
        .dispatch_typed(&support::TypedEvent {
            owner: ModuleId::new(11),
            value: u8::MAX - 1,
        })
        .unwrap();
    runtime.update(seconds(0.0)).unwrap();
    runtime.update(seconds(0.25)).unwrap();
    let after = runtime
        .layer(LayerId::new(0))
        .unwrap()
        .current_payload_event_tracks()[0]
        .as_ref()
        .unwrap()
        .playback_seconds();
    assert_eq!(after, before);
    assert_eq!(runtime.modules().payload_event_updates.len(), update_count);
}

#[test]
fn opaque_gate_false_skips_primary_and_aligned_payload_playback() {
    let track = EventTrackDefinition::new(vec![interval(9, false, 0.5, 0.0, Vec::new())]).unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    assert_eq!(runtime.functions().current_event_stops.len(), 1);

    runtime.update(seconds(0.25)).unwrap();
    assert!(runtime.functions().current_event_updates.is_empty());
    runtime.functions_mut().current_event_gate_open = true;
    runtime.update(seconds(0.1)).unwrap();
    assert_eq!(runtime.functions().current_event_updates.len(), 1);
}

// The replaced step is the exact 0.5 the fixture handed the host back.
#[allow(clippy::float_cmp)]
#[test]
fn first_external_primary_replaces_layer_step_once() {
    let track = EventTrackDefinition::new(vec![interval(12, true, 0.0, 0.0, Vec::new())]).unwrap();
    let mut runtime = runtime(
        vec![with_executable_tracks(sequence(1.0, false), vec![track])],
        vec![event_layer()],
    );
    runtime
        .trans(
            LayerId::new(0),
            TransitionRequest::immediate(Some(SequenceId::new(0))),
        )
        .unwrap();
    runtime.functions_mut().current_event_step = Some(0.5);

    runtime.update(seconds(0.1)).unwrap();
    assert_eq!(
        runtime
            .layer(LayerId::new(0))
            .unwrap()
            .current_time_seconds(),
        0.5
    );
    assert_eq!(
        runtime.functions().current_event_updates[0].delta_seconds,
        0.5
    );
}
