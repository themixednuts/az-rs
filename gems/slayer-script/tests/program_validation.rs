use std::sync::Arc;

use az_gem_slayer_script::{
    AuthoredEventGroupCount, BoundEventProperties, CurrentEventHostExecution,
    EventIntervalDefinition, EventRootDefinition, EventTrackDefinition,
    ExecutableEventChannelCount, ExecutableEventId, LayerDefinition, LayerId,
    ProgramValidationError, SequenceDefinition, SequenceId, SlayerProgram, SlayerRuntime,
    StateTable, TransitionOutcome, TransitionRequest,
};

mod support;

use support::{TestCallback, TestFunctions, TestModules, TestOperation};

#[test]
fn program_rejects_invalid_references_and_zero_duration_wrapping() {
    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![SequenceDefinition::new(support::seconds(0.0), true)],
            Vec::new(),
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::ZeroDurationWrappingSequence {
            sequence: SequenceId::new(0)
        }
    );

    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![SequenceDefinition::new(support::seconds(1.0), false)],
            vec![LayerDefinition::new().with_sequences(vec![SequenceId::new(9)])],
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::UnknownLayerSequence {
            layer: LayerId::new(0),
            sequence: SequenceId::new(9),
        }
    );
}

#[test]
fn program_rejects_unknown_and_cyclic_sequence_parents() {
    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![
                SequenceDefinition::new(support::seconds(1.0), false)
                    .with_parent_sequence(SequenceId::new(9)),
            ],
            Vec::new(),
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::UnknownSequenceParent {
            sequence: SequenceId::new(0),
            parent: SequenceId::new(9),
        }
    );

    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![
                SequenceDefinition::new(support::seconds(1.0), false)
                    .with_parent_sequence(SequenceId::new(1)),
                SequenceDefinition::new(support::seconds(1.0), false)
                    .with_parent_sequence(SequenceId::new(0)),
            ],
            Vec::new(),
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::SequenceParentCycle {
            sequence: SequenceId::new(0),
        }
    );
}

#[test]
fn executable_tracks_require_positive_duration_and_explicit_layer_capacity() {
    let zero_interval = EventIntervalDefinition::new(
        support::seconds(0.0),
        support::seconds(1.0),
        EventRootDefinition::new(ExecutableEventId::new(7), support::seconds(0.0), false).unwrap(),
        BoundEventProperties::new(1.0, 0.0, 0.0, 0.0, 1.0, false).unwrap(),
        Vec::new(),
    )
    .unwrap();
    let zero_track = EventTrackDefinition::new(vec![zero_interval]).unwrap();
    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![
                SequenceDefinition::new(support::seconds(1.0), false)
                    .with_authored_primary_event_group_count(AuthoredEventGroupCount::new(1))
                    .with_executable_event_tracks(vec![zero_track])
            ],
            vec![
                LayerDefinition::new()
                    .with_executable_event_channel_count(
                        ExecutableEventChannelCount::new(1).unwrap()
                    )
                    .with_sequences(vec![SequenceId::new(0)])
            ],
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::ZeroDurationExecutableEvent {
            sequence: SequenceId::new(0),
            group_index: 0,
        }
    );

    let interval = EventIntervalDefinition::new(
        support::seconds(0.0),
        support::seconds(1.0),
        EventRootDefinition::new(ExecutableEventId::new(7), support::seconds(1.0), false).unwrap(),
        BoundEventProperties::new(1.0, 0.0, 0.0, 0.0, 1.0, false).unwrap(),
        Vec::new(),
    )
    .unwrap();
    let track = EventTrackDefinition::new(vec![interval]).unwrap();
    assert_eq!(
        SlayerProgram::<TestOperation, TestCallback>::new(
            vec![
                SequenceDefinition::new(support::seconds(1.0), false)
                    .with_authored_primary_event_group_count(AuthoredEventGroupCount::new(1))
                    .with_executable_event_tracks(vec![track])
            ],
            vec![LayerDefinition::new().with_sequences(vec![SequenceId::new(0)])],
            StateTable::empty(),
        )
        .unwrap_err(),
        ProgramValidationError::InsufficientExecutableEventChannels {
            layer: LayerId::new(0),
            count: 0,
            required: 1,
        }
    );
}

#[test]
fn instances_start_empty_and_share_only_immutable_program_tables() {
    let program = Arc::new(
        SlayerProgram::new(
            vec![
                SequenceDefinition::<TestOperation, TestCallback>::new(
                    support::seconds(1.0),
                    false,
                ),
                SequenceDefinition::new(support::seconds(1.0), false),
            ],
            vec![
                LayerDefinition::new().with_sequences(vec![SequenceId::new(0), SequenceId::new(1)]),
            ],
            StateTable::empty(),
        )
        .unwrap(),
    );
    let mut first = SlayerRuntime::new(
        Arc::clone(&program),
        TestModules::default(),
        TestFunctions::default(),
        CurrentEventHostExecution::Enabled,
    );
    let second = SlayerRuntime::new(
        program,
        TestModules::default(),
        TestFunctions::default(),
        CurrentEventHostExecution::Enabled,
    );

    assert_eq!(first.layer(LayerId::new(0)).unwrap().current(), None);
    assert_eq!(second.layer(LayerId::new(0)).unwrap().current(), None);
    assert!(matches!(
        first
            .trans(
                LayerId::new(0),
                TransitionRequest::immediate(Some(SequenceId::new(1))),
            )
            .unwrap(),
        TransitionOutcome::Applied(_)
    ));

    assert_eq!(
        first.layer(LayerId::new(0)).unwrap().current(),
        Some(SequenceId::new(1))
    );
    assert_eq!(second.layer(LayerId::new(0)).unwrap().current(), None);
}
