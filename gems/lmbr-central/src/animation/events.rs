use std::sync::Arc;

use az_animation::events::{AnimationEventData, AnimationEventDatabaseAsset};
use az_core::{AssetData, AssetId, AzTypeInfo};
use az_framework::asset::AssetCatalog;
use az_gem_animation::{BevyTransitionAnimation, CryAnimationPlayer, CryAnimationSet};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

pub const ANIMATION_EVENT_TYPE_ID: Uuid = uuid!("1d927664-8c19-4ea9-a59f-23a81ec486f2");

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEvent {
    pub time: f32,
    pub end_time: f32,
    pub animation_name: Arc<str>,
    pub event_name: Arc<str>,
    pub parameter: Arc<str>,
    pub bone_name_1: Arc<str>,
    pub bone_name_2: Arc<str>,
    pub offset: Vec3,
    pub direction: Vec3,
}

impl AnimationEvent {
    #[must_use]
    pub fn duration(&self) -> f32 {
        (self.end_time - self.time).max(0.0)
    }
}

impl AzTypeInfo for AnimationEvent {
    const NAME: &'static str = "AnimationEvent";
    const TYPE_ID: Uuid = ANIMATION_EVENT_TYPE_ID;
}

#[derive(Message, Debug, Clone, PartialEq)]
pub struct CharacterAnimationEvent {
    pub character: Entity,
    pub layer: u32,
    pub animation: az_animation::character::AnimationInstanceId,
    pub event: AnimationEvent,
}

#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CharacterAnimationEventDatabase {
    pub asset: AssetId,
}

#[derive(Component, Debug, Clone)]
struct ResolvedCharacterAnimationEventDatabase {
    asset: AssetId,
    handle: Handle<AnimationEventDatabaseAsset>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharacterAnimationEventSet {
    Resolve,
    Dispatch,
}

pub struct CharacterAnimationEventsPlugin;

impl Plugin for CharacterAnimationEventsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<CharacterAnimationEventDatabase>()
            .add_message::<CharacterAnimationEvent>()
            .configure_sets(
                Update,
                (
                    CharacterAnimationEventSet::Resolve,
                    CharacterAnimationEventSet::Dispatch,
                )
                    .chain()
                    .after(CryAnimationSet::Advance),
            )
            .add_systems(
                Update,
                (
                    resolve_character_animation_event_databases
                        .in_set(CharacterAnimationEventSet::Resolve),
                    dispatch_character_animation_events
                        .in_set(CharacterAnimationEventSet::Dispatch),
                ),
            );
    }
}

/// Entities whose animation-event database binding still needs resolving.
type UnresolvedEventDatabases = (
    Entity,
    &'static CharacterAnimationEventDatabase,
    Option<&'static ResolvedCharacterAnimationEventDatabase>,
);

/// Only bindings that changed, or that never resolved, are worth revisiting.
type UnresolvedEventDatabaseFilter = Or<(
    Changed<CharacterAnimationEventDatabase>,
    Without<ResolvedCharacterAnimationEventDatabase>,
)>;

fn resolve_character_animation_event_databases(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    catalog: Option<Res<AssetCatalog>>,
    bindings: Query<UnresolvedEventDatabases, UnresolvedEventDatabaseFilter>,
) {
    let (Some(asset_server), Some(catalog)) = (asset_server, catalog) else {
        return;
    };
    for (entity, binding, resolved) in &bindings {
        if resolved.is_some_and(|resolved| resolved.asset == binding.asset) {
            continue;
        }
        let Some(entry) = catalog.get_by_id(binding.asset) else {
            tracing::warn!(asset = %binding.asset, "animation-event database is absent from the asset catalog");
            continue;
        };
        if entry.asset_type() != AnimationEventDatabaseAsset::ASSET_TYPE {
            tracing::warn!(
                asset = %binding.asset,
                actual = %entry.asset_type(),
                "animation-event database catalog type mismatch"
            );
            continue;
        }
        commands
            .entity(entity)
            .insert(ResolvedCharacterAnimationEventDatabase {
                asset: binding.asset,
                handle: asset_server.load(entry.relative_path().to_path_buf()),
            });
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn dispatch_character_animation_events(
    databases: Res<Assets<AnimationEventDatabaseAsset>>,
    characters: Query<(
        Entity,
        &CryAnimationPlayer,
        &ResolvedCharacterAnimationEventDatabase,
    )>,
    mut events: MessageWriter<CharacterAnimationEvent>,
) {
    for (character, player, binding) in &characters {
        let Some(database) = databases.get(&binding.handle) else {
            continue;
        };
        // Counting the layer index in `u32` avoids a `usize` -> `u32` cast; the
        // player has a handful of layers so the range is never exhausted.
        for (layer, queue) in (0u32..).zip(player.runtime().layers()) {
            for animation in queue.active_animations() {
                dispatch_animation_events(character, layer, animation, database, &mut events);
            }
        }
    }
}

fn dispatch_animation_events(
    character: Entity,
    layer: u32,
    animation: &BevyTransitionAnimation,
    database: &AnimationEventDatabaseAsset,
    output: &mut MessageWriter<CharacterAnimationEvent>,
) {
    let Some(motion) = animation.animation().asset_id() else {
        return;
    };
    let Some(track) = database.track(motion) else {
        return;
    };
    let Some(window) = animation.animation().event_window(animation) else {
        return;
    };
    visit_event_window(&track.events, window, |event| {
        output.write(CharacterAnimationEvent {
            character,
            layer,
            animation: animation.id(),
            event: AnimationEvent {
                time: event.time,
                end_time: event.end_time,
                animation_name: track.animation_name.clone(),
                event_name: event.name.clone(),
                parameter: event.parameter.clone(),
                bone_name_1: event.bone_name_1.clone(),
                bone_name_2: event.bone_name_2.clone(),
                offset: event.offset,
                direction: event.direction,
            },
        });
    });
}

fn visit_event_window(
    event_data: &[AnimationEventData],
    window: az_gem_animation::MotionEventWindow,
    mut visit: impl FnMut(&AnimationEventData),
) {
    // `previous` is the value `current` held on the last update, copied
    // verbatim, so bit equality is exactly the "clip did not advance" test an
    // epsilon comparison would blur into skipping real sub-epsilon windows.
    #[allow(
        clippy::float_cmp,
        reason = "exact equality is the intended no-advance sentinel; see the comment above"
    )]
    if window.cycles == 0 && window.previous == window.current {
        return;
    }
    if window.cycles == 0 && window.previous <= window.current {
        visit_interval(
            event_data,
            window.previous,
            window.current,
            window.include_start,
            &mut visit,
        );
        return;
    }

    visit_interval(
        event_data,
        window.previous,
        1.0,
        window.include_start,
        &mut visit,
    );
    for _ in 0..window.cycles.saturating_sub(1) {
        visit_interval(event_data, 0.0, 1.0, true, &mut visit);
    }
    visit_interval(event_data, 0.0, window.current, true, &mut visit);
}

fn visit_interval<'a>(
    event_data: &'a [AnimationEventData],
    start: f32,
    end: f32,
    include_start: bool,
    visit: &mut impl FnMut(&'a AnimationEventData),
) {
    for event in event_data {
        let after_start = if include_start {
            event.time >= start
        } else {
            event.time > start
        };
        if !after_start || event.time > end {
            continue;
        }
        visit(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_animation::MotionEventWindow;

    fn event(time: f32, name: &'static str) -> AnimationEventData {
        AnimationEventData {
            time,
            end_time: time,
            name: name.into(),
            parameter: Arc::from(""),
            bone_name_1: Arc::from(""),
            bone_name_2: Arc::from(""),
            offset: Vec3::ZERO,
            direction: Vec3::ZERO,
            model: Arc::from(""),
        }
    }

    fn names(events: &[AnimationEventData], window: MotionEventWindow) -> Vec<Arc<str>> {
        let mut names = Vec::new();
        visit_event_window(events, window, |event| names.push(event.name.clone()));
        names
    }

    #[test]
    fn first_evaluation_includes_the_start_key() {
        let events = [event(0.0, "start"), event(0.5, "middle")];
        assert_eq!(
            names(
                &events,
                MotionEventWindow {
                    previous: 0.0,
                    current: 0.5,
                    cycles: 0,
                    include_start: true,
                },
            ),
            [Arc::<str>::from("start"), Arc::<str>::from("middle")]
        );
        assert_eq!(
            names(
                &events,
                MotionEventWindow {
                    previous: 0.0,
                    current: 0.5,
                    cycles: 0,
                    include_start: false,
                },
            ),
            [Arc::<str>::from("middle")]
        );
    }

    #[test]
    fn every_crossed_cycle_dispatches_its_events() {
        let events = [event(0.1, "early"), event(0.9, "late")];
        assert_eq!(
            names(
                &events,
                MotionEventWindow {
                    previous: 0.8,
                    current: 0.2,
                    cycles: 2,
                    include_start: false,
                },
            ),
            [
                Arc::<str>::from("late"),
                Arc::<str>::from("early"),
                Arc::<str>::from("late"),
                Arc::<str>::from("early"),
            ]
        );
    }

    #[test]
    fn stationary_windows_do_not_dispatch() {
        assert!(
            names(
                &[event(0.5, "middle")],
                MotionEventWindow {
                    previous: 0.5,
                    current: 0.5,
                    cycles: 0,
                    include_start: true,
                },
            )
            .is_empty()
        );
    }
}
