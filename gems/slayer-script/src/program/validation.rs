//! Validation and construction of immutable compiled program tables.

use thiserror::Error;

use super::{DurationSeconds, LayerDefinition, LayerKind, SequenceDefinition, SlayerProgram};
use crate::{LayerId, SequenceId, StateTable};

impl<O, E> SlayerProgram<O, E> {
    /// Validates dense table references and looping duration invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ProgramValidationError::TooManySequences`] or
    /// [`ProgramValidationError::TooManyLayers`] when a dense `u32` identifier
    /// cannot address the table, and
    /// [`ProgramValidationError::MultipleAuxiliaryLayers`] when more than one
    /// auxiliary layer is compiled. Per-sequence checks then reject unaligned
    /// payload tracks and zero-duration event roots or wrapping sequences; the
    /// parent walk rejects an unknown or cyclic sequence parent; and channel
    /// assignment rejects a layer that binds an unknown sequence, reserves
    /// fewer executable event channels than a bound sequence needs, or pushes
    /// the cumulative channel base past `i32`.
    pub fn new(
        sequences: impl Into<Box<[SequenceDefinition<O, E>]>>,
        layers: impl Into<Box<[LayerDefinition]>>,
        states: StateTable<O>,
    ) -> Result<Self, ProgramValidationError> {
        let sequences = sequences.into();
        let mut layers = layers.into();
        if u32::try_from(sequences.len()).is_err() {
            return Err(ProgramValidationError::TooManySequences {
                count: sequences.len(),
            });
        }
        if u32::try_from(layers.len()).is_err() {
            return Err(ProgramValidationError::TooManyLayers {
                count: layers.len(),
            });
        }
        let auxiliary_count = layers
            .iter()
            .filter(|layer| layer.kind() == LayerKind::Auxiliary)
            .count();
        if auxiliary_count > 1 {
            return Err(ProgramValidationError::MultipleAuxiliaryLayers {
                count: auxiliary_count,
            });
        }
        for (index, sequence) in (0..u32::MAX).zip(sequences.iter()) {
            validate_sequence(SequenceId::new(index), sequence)?;
        }
        validate_sequence_parents(&sequences)?;
        assign_channel_bases(&mut layers, &sequences)?;
        Ok(Self {
            sequences,
            layers,
            states,
        })
    }
}

/// Checks one compiled sequence's own track alignment and duration invariants.
///
/// # Errors
///
/// Returns [`ProgramValidationError::ExecutableTracksExceedAuthoredGroups`],
/// [`ProgramValidationError::UnalignedPayloadEventTracks`], or
/// [`ProgramValidationError::UnalignedPayloadEventIntervals`] when the payload
/// tracks are not index-aligned with their primary groups, and
/// [`ProgramValidationError::ZeroDurationExecutableEvent`],
/// [`ProgramValidationError::ZeroDurationPayloadEvent`], or
/// [`ProgramValidationError::ZeroDurationWrappingSequence`] when playback
/// would have to normalize by a zero duration.
///
/// # Panics
///
/// Panics if a `u32` authored event-group count does not fit `usize`.
fn validate_sequence<O, E>(
    sequence_id: SequenceId,
    sequence: &SequenceDefinition<O, E>,
) -> Result<(), ProgramValidationError> {
    let executable_count = sequence.executable_event_tracks().len();
    let authored_count = usize::try_from(sequence.authored_primary_event_group_count().get())
        .expect("u32 authored event-group counts must fit usize");
    if executable_count > authored_count {
        return Err(
            ProgramValidationError::ExecutableTracksExceedAuthoredGroups {
                sequence: sequence_id,
                executable: executable_count,
                authored: authored_count,
            },
        );
    }
    if sequence.executable_payload_event_tracks().len() > executable_count {
        return Err(ProgramValidationError::UnalignedPayloadEventTracks {
            sequence: sequence_id,
        });
    }
    for (group_index, payload) in sequence
        .executable_payload_event_tracks()
        .iter()
        .enumerate()
    {
        if let Some(payload) = payload.executable_track()
            && payload.intervals().len()
                != sequence.executable_event_tracks()[group_index]
                    .intervals()
                    .len()
        {
            return Err(ProgramValidationError::UnalignedPayloadEventIntervals {
                sequence: sequence_id,
                group_index,
            });
        }
    }
    for (group_index, group) in sequence.executable_event_tracks().iter().enumerate() {
        if group
            .intervals()
            .iter()
            .any(|interval| interval.event_duration() == DurationSeconds::ZERO)
        {
            return Err(ProgramValidationError::ZeroDurationExecutableEvent {
                sequence: sequence_id,
                group_index,
            });
        }
    }
    for (group_index, group) in sequence
        .executable_payload_event_tracks()
        .iter()
        .enumerate()
    {
        if group.executable_track().is_some_and(|group| {
            group
                .intervals()
                .iter()
                .any(|interval| interval.event_duration() == DurationSeconds::ZERO)
        }) {
            return Err(ProgramValidationError::ZeroDurationPayloadEvent {
                sequence: sequence_id,
                group_index,
            });
        }
    }
    if (sequence.is_looping() || sequence.wraps_non_looping_at_end())
        && sequence.duration() == DurationSeconds::ZERO
    {
        return Err(ProgramValidationError::ZeroDurationWrappingSequence {
            sequence: sequence_id,
        });
    }
    Ok(())
}

/// Walks every sequence's parent chain to its root.
///
/// # Errors
///
/// Returns [`ProgramValidationError::UnknownSequenceParent`] when a chain
/// names a sequence outside the dense table, or
/// [`ProgramValidationError::SequenceParentCycle`] when the walk revisits a
/// sequence and would therefore never terminate.
fn validate_sequence_parents<O, E>(
    sequences: &[SequenceDefinition<O, E>],
) -> Result<(), ProgramValidationError> {
    for (index, sequence) in (0..u32::MAX).zip(sequences.iter()) {
        let sequence_id = SequenceId::new(index);
        let mut next = sequence.parent_sequence();
        let mut visited = std::collections::BTreeSet::new();
        visited.insert(sequence_id);
        while let Some(parent) = next {
            let Some(parent_definition) = sequences.get(parent.index()) else {
                return Err(ProgramValidationError::UnknownSequenceParent {
                    sequence: sequence_id,
                    parent,
                });
            };
            if !visited.insert(parent) {
                return Err(ProgramValidationError::SequenceParentCycle {
                    sequence: sequence_id,
                });
            }
            next = parent_definition.parent_sequence();
        }
    }
    Ok(())
}

/// Assigns each layer its cumulative native executable-channel base.
///
/// # Errors
///
/// Returns [`ProgramValidationError::UnknownLayerSequence`] when a layer binds
/// a sequence outside the dense table,
/// [`ProgramValidationError::InsufficientExecutableEventChannels`] when a
/// normal layer reserves fewer channels than its widest bound sequence needs,
/// or [`ProgramValidationError::ExecutableEventChannelOverflow`] when the
/// running base leaves `i32`.
///
/// # Panics
///
/// Panics if a `u32` authored event-group count or a validated nonnegative
/// `i32` channel count does not fit `usize`.
fn assign_channel_bases<O, E>(
    layers: &mut [LayerDefinition],
    sequences: &[SequenceDefinition<O, E>],
) -> Result<(), ProgramValidationError> {
    let mut channel_base = 0_i32;
    for (index, layer) in (0..u32::MAX).zip(layers.iter_mut()) {
        layer.executable_event_channel_base = channel_base;
        let count = layer.executable_event_channel_count().get();
        let mut required = 0_usize;
        for &sequence_id in layer.sequences() {
            let Some(sequence) = sequences.get(sequence_id.index()) else {
                return Err(ProgramValidationError::UnknownLayerSequence {
                    layer: LayerId::new(index),
                    sequence: sequence_id,
                });
            };
            let authored = usize::try_from(sequence.authored_primary_event_group_count().get())
                .expect("u32 authored event-group counts must fit usize");
            required = required
                .max(authored)
                .max(sequence.executable_event_tracks().len());
        }
        if layer.kind() == LayerKind::Normal
            && usize::try_from(count).expect("nonnegative i32 must fit usize") < required
        {
            return Err(
                ProgramValidationError::InsufficientExecutableEventChannels {
                    layer: LayerId::new(index),
                    count,
                    required,
                },
            );
        }
        channel_base = channel_base
            .checked_add(count)
            .ok_or(ProgramValidationError::ExecutableEventChannelOverflow)?;
    }
    Ok(())
}

/// Why immutable `SlayerScript` program tables are invalid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProgramValidationError {
    /// Dense sequence identifiers cannot represent the table.
    #[error("program contains {count} sequences, exceeding the u32 identifier space")]
    TooManySequences { count: usize },
    /// Dense layer identifiers cannot represent the table.
    #[error("program contains {count} layers, exceeding the u32 identifier space")]
    TooManyLayers { count: usize },
    /// Native runtime contains at most one auxiliary layer.
    #[error("program contains {count} auxiliary layers; native runtime supports one")]
    MultipleAuxiliaryLayers { count: usize },
    /// Payload groups must be index-aligned with a primary channel.
    #[error("sequence {sequence:?} has payload groups without matching primary groups")]
    UnalignedPayloadEventTracks { sequence: SequenceId },
    /// Each payload group must use the primary group's interval index table.
    #[error(
        "sequence {sequence:?} payload group {group_index} is not interval-aligned with primary"
    )]
    UnalignedPayloadEventIntervals {
        sequence: SequenceId,
        group_index: usize,
    },
    /// A compiled current slot must correspond to an authored primary group.
    #[error(
        "sequence {sequence:?} has {executable} executable slots but only {authored} authored primary groups"
    )]
    ExecutableTracksExceedAuthoredGroups {
        sequence: SequenceId,
        executable: usize,
        authored: usize,
    },
    /// A bound sequence requires more channels than its compiled layer reserves.
    #[error("layer {layer:?} reserves {count} executable event channels but requires {required}")]
    InsufficientExecutableEventChannels {
        layer: LayerId,
        count: i32,
        required: usize,
    },
    /// Cumulative signed native channel bases overflowed.
    #[error("cumulative executable event channel bases exceed i32")]
    ExecutableEventChannelOverflow,
    /// Fixed current-layer playback normalizes by event-root duration.
    #[error("sequence {sequence:?} executable group {group_index} has zero event duration")]
    ZeroDurationExecutableEvent {
        sequence: SequenceId,
        group_index: usize,
    },
    /// Fixed payload playback normalizes by event-root duration.
    #[error("sequence {sequence:?} payload group {group_index} has zero event duration")]
    ZeroDurationPayloadEvent {
        sequence: SequenceId,
        group_index: usize,
    },
    /// Wrapping time cannot apply native modulo at zero duration.
    #[error("wrapping sequence {sequence:?} has zero duration")]
    ZeroDurationWrappingSequence { sequence: SequenceId },
    /// A layer's compiler-owned sequence table references no program sequence.
    #[error("layer {layer:?} binds unknown sequence {sequence:?}")]
    UnknownLayerSequence {
        layer: LayerId,
        sequence: SequenceId,
    },
    /// A compiled sequence parent is absent from the dense program table.
    #[error("sequence {sequence:?} references unknown parent {parent:?}")]
    UnknownSequenceParent {
        sequence: SequenceId,
        parent: SequenceId,
    },
    /// Sequence-parent traversal would recurse forever.
    #[error("sequence {sequence:?} participates in a parent cycle")]
    SequenceParentCycle { sequence: SequenceId },
}
