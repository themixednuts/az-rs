use bevy::prelude::*;

use crate::{AudioControlId, AudioObstructionType};

/// Runtime request handled by an `AudioSystem` backend.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum AudioRequest {
    ExecuteTrigger {
        entity: Entity,
        trigger_id: AudioControlId,
        trigger_name: String,
        notify_when_finished: bool,
    },
    KillTrigger {
        entity: Entity,
        trigger_id: AudioControlId,
        trigger_name: String,
    },
    KillAllTriggers {
        entity: Entity,
    },
    SetRtpcValue {
        entity: Entity,
        rtpc_id: AudioControlId,
        rtpc_name: String,
        value: f32,
    },
    SetSwitchState {
        entity: Entity,
        switch_id: AudioControlId,
        switch_name: String,
        state_id: AudioControlId,
        state_name: String,
    },
    LoadPreload {
        entity: Entity,
        preload_id: AudioControlId,
        preload_name: String,
    },
    UnloadPreload {
        entity: Entity,
        preload_id: AudioControlId,
        preload_name: String,
    },
    SetEnvironmentAmount {
        entity: Entity,
        environment_id: AudioControlId,
        environment_name: String,
        amount: f32,
    },
    SetListenerEnabled {
        entity: Entity,
        enabled: bool,
    },
    SetListenerTransform {
        entity: Entity,
        transform: Transform,
    },
    SetObstructionType {
        entity: Entity,
        obstruction_type: AudioObstructionType,
    },
    SetMovesWithEntity {
        entity: Entity,
        tracks_entity_position: bool,
    },
}
