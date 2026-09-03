use az_core::EntityId;
use bevy::prelude::*;

use super::{ParticleComponent, ParticleEmitterSettings};

/// A mutation or one-shot operation addressed to one particle component.
#[derive(Debug, Clone, PartialEq, Message)]
pub struct ParticleComponentRequest {
    pub entity: Entity,
    pub action: ParticleComponentAction,
}

impl ParticleComponentRequest {
    #[must_use]
    pub const fn new(entity: Entity, action: ParticleComponentAction) -> Self {
        Self { entity, action }
    }

    #[must_use]
    pub const fn enable(entity: Entity, enabled: bool) -> Self {
        Self::new(entity, ParticleComponentAction::Enable(enabled))
    }

    #[must_use]
    pub const fn enable_audio(entity: Entity, enabled: bool) -> Self {
        Self::new(entity, ParticleComponentAction::EnableAudio(enabled))
    }

    #[must_use]
    pub const fn show(entity: Entity) -> Self {
        Self::new(entity, ParticleComponentAction::Show)
    }

    #[must_use]
    pub const fn hide(entity: Entity) -> Self {
        Self::new(entity, ParticleComponentAction::Hide)
    }

    #[must_use]
    pub const fn set_visibility(entity: Entity, visible: bool) -> Self {
        Self::new(entity, ParticleComponentAction::SetVisibility(visible))
    }

    #[must_use]
    pub const fn set_alpha_scale(entity: Entity, scale: f32) -> Self {
        Self::new(entity, ParticleComponentAction::SetAlphaScale(scale))
    }

    #[must_use]
    pub const fn set_count_scale(entity: Entity, scale: f32) -> Self {
        Self::new(entity, ParticleComponentAction::SetCountScale(scale))
    }

    #[must_use]
    pub const fn set_global_size_scale(entity: Entity, scale: f32) -> Self {
        Self::new(entity, ParticleComponentAction::SetGlobalSizeScale(scale))
    }

    #[must_use]
    pub fn set_rtpc(entity: Entity, rtpc: impl Into<String>) -> Self {
        Self::new(entity, ParticleComponentAction::SetRtpc(rtpc.into()))
    }
}

/// Complete runtime request surface for [`ParticleComponent`].
#[derive(Debug, Clone, PartialEq)]
pub enum ParticleComponentAction {
    Enable(bool),
    EnableAudio(bool),
    EnablePreRoll(bool),
    EmitPulse,
    Hide,
    Restart,
    SetupEmitter {
        emitter_name: String,
        settings: Box<ParticleEmitterSettings>,
    },
    Show,
    SetAlphaScale(f32),
    SetColorTint(LinearRgba),
    SetCountScale(f32),
    SetGlobalSizeScale(f32),
    SetIgnoreRotation(bool),
    SetLifetimeStrength(f32),
    SetNotAttached(bool),
    SetParticleSizeScaleX(f32),
    SetParticleSizeScaleY(f32),
    SetParticleSizeScaleZ(f32),
    SetParticleSizeScaleRandom(f32),
    SetPulsePeriod(f32),
    SetRtpc(String),
    SetSpeedScale(f32),
    SetTargetEntity(EntityId),
    SetTimeScale(f32),
    SetUseBoundingBox(bool),
    SetUseLod(bool),
    SetUseVisArea(bool),
    SetViewDistanceMultiplier(f32),
    SetVisibility(bool),
}

/// One-shot operation for a concrete particle backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub struct ParticleEmitterTrigger {
    pub entity: Entity,
    pub kind: ParticleEmitterTriggerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleEmitterTriggerKind {
    /// Releases the component-owned emitter during entity deactivation.
    /// Backends retain any surviving particles independently of the ECS entity.
    Deactivate {
        hide: bool,
        kill: bool,
    },
    EmitPulse,
    Restart,
}

fn replace_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

fn replace_bounded(target: &mut f32, value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value) && replace_if_changed(target, value)
}

impl ParticleComponentAction {
    fn apply(&self, settings: &mut ParticleEmitterSettings) -> ParticleComponentActionOutcome {
        let changed = match self {
            Self::Enable(value) => replace_if_changed(&mut settings.enable, *value),
            Self::EnableAudio(value) => replace_if_changed(&mut settings.enable_audio, *value),
            Self::EnablePreRoll(value) => replace_if_changed(&mut settings.pre_roll, *value),
            Self::EmitPulse => {
                return ParticleComponentActionOutcome::trigger(
                    ParticleEmitterTriggerKind::EmitPulse,
                );
            }
            Self::Hide => replace_if_changed(&mut settings.visible, false),
            Self::Restart => {
                return ParticleComponentActionOutcome::trigger(
                    ParticleEmitterTriggerKind::Restart,
                );
            }
            Self::SetupEmitter {
                emitter_name,
                settings: replacement,
            } => {
                let mut replacement = (**replacement).clone();
                replacement.selected_emitter.clone_from(emitter_name);
                replace_if_changed(settings, replacement)
            }
            Self::Show => replace_if_changed(&mut settings.visible, true),
            Self::SetAlphaScale(value) => {
                if value.is_finite() {
                    replace_if_changed(&mut settings.alpha_scale, *value)
                } else {
                    false
                }
            }
            Self::SetColorTint(value) => replace_if_changed(&mut settings.color, *value),
            Self::SetCountScale(value) => {
                replace_bounded(&mut settings.particle_count_scale, *value, 0.0, 1_000.0)
            }
            Self::SetGlobalSizeScale(value) => {
                replace_bounded(&mut settings.global_size_scale, *value, 0.0, 100.0)
            }
            Self::SetIgnoreRotation(value) => {
                replace_if_changed(&mut settings.ignore_rotation, *value)
            }
            Self::SetLifetimeStrength(value) => {
                replace_bounded(&mut settings.strength, *value, -1.0, 1.0)
            }
            Self::SetNotAttached(value) => replace_if_changed(&mut settings.not_attached, *value),
            Self::SetParticleSizeScaleX(value) => {
                replace_bounded(&mut settings.particle_size_x, *value, 0.0, 100.0)
            }
            Self::SetParticleSizeScaleY(value) => {
                replace_bounded(&mut settings.particle_size_y, *value, 0.0, 100.0)
            }
            Self::SetParticleSizeScaleZ(value) => {
                replace_bounded(&mut settings.particle_size_z, *value, 0.0, 100.0)
            }
            Self::SetParticleSizeScaleRandom(value) => {
                replace_bounded(&mut settings.particle_size_random, *value, 0.0, 1.0)
            }
            Self::SetPulsePeriod(value) => {
                replace_bounded(&mut settings.pulse_period, *value, 0.0, f32::MAX)
            }
            Self::SetRtpc(value) => {
                if settings.audio_rtpc == *value {
                    false
                } else {
                    settings.audio_rtpc.clone_from(value);
                    true
                }
            }
            Self::SetSpeedScale(value) => {
                replace_bounded(&mut settings.speed_scale, *value, 0.0, 1_000.0)
            }
            Self::SetTargetEntity(value) => replace_if_changed(&mut settings.target_entity, *value),
            Self::SetTimeScale(value) => {
                replace_bounded(&mut settings.time_scale, *value, 0.0, 1_000.0)
            }
            Self::SetUseBoundingBox(value) => {
                replace_if_changed(&mut settings.register_by_bounding_box, *value)
            }
            Self::SetUseLod(value) => replace_if_changed(&mut settings.use_lod, *value),
            Self::SetUseVisArea(value) => replace_if_changed(&mut settings.use_vis_area, *value),
            Self::SetViewDistanceMultiplier(value) => {
                if value.is_finite() {
                    replace_if_changed(&mut settings.view_distance_multiplier, *value)
                } else {
                    false
                }
            }
            Self::SetVisibility(value) => replace_if_changed(&mut settings.visible, *value),
        };
        ParticleComponentActionOutcome {
            changed,
            trigger: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParticleComponentActionOutcome {
    changed: bool,
    trigger: Option<ParticleEmitterTriggerKind>,
}

impl ParticleComponentActionOutcome {
    const fn trigger(trigger: ParticleEmitterTriggerKind) -> Self {
        Self {
            changed: false,
            trigger: Some(trigger),
        }
    }
}

pub(super) fn route_particle_component_requests(
    mut requests: MessageReader<ParticleComponentRequest>,
    mut components: Query<&mut ParticleComponent>,
    mut triggers: MessageWriter<ParticleEmitterTrigger>,
) {
    for request in requests.read() {
        let Ok(mut component) = components.get_mut(request.entity) else {
            continue;
        };
        let outcome = request
            .action
            .apply(&mut component.bypass_change_detection().settings);
        if outcome.changed {
            component.set_changed();
        }
        if let Some(kind) = outcome.trigger {
            triggers.write(ParticleEmitterTrigger {
                entity: request.entity,
                kind,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn bounded_updates_reject_out_of_range_values() {
        let mut settings = ParticleEmitterSettings::default();
        ParticleComponentAction::SetCountScale(1_001.0).apply(&mut settings);
        ParticleComponentAction::SetParticleSizeScaleRandom(-0.1).apply(&mut settings);
        ParticleComponentAction::SetLifetimeStrength(f32::NAN).apply(&mut settings);

        assert_eq!(settings.particle_count_scale, 1.0);
        assert_eq!(settings.particle_size_random, 0.0);
        assert_eq!(settings.strength, -1.0);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn setup_emitter_uses_explicit_name() {
        let mut settings = ParticleEmitterSettings::default();
        let replacement = ParticleEmitterSettings {
            selected_emitter: "ignored".to_owned(),
            time_scale: 2.0,
            ..Default::default()
        };
        ParticleComponentAction::SetupEmitter {
            emitter_name: "effects/sparks".to_owned(),
            settings: Box::new(replacement),
        }
        .apply(&mut settings);

        assert_eq!(settings.selected_emitter, "effects/sparks");
        assert_eq!(settings.time_scale, 2.0);
    }
}
