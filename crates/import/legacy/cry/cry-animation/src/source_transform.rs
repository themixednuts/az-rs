//! Legacy `CryAnimation` event-list XML import transform.

use az_animation::events::{
    AnimationEventListSource, AnimationEventSource, AnimationEventVec3, AnimationEventsSource,
};
use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
    normalize_source_path,
};

use crate::{AnimationEventItem, AnimationEventRef, ParseError, visit_animation_event_list};

pub const ANIMATION_EVENT_LIST_SOURCE_SCHEMA: SourceSchemaType =
    az_animation::source_schemas::ANIMATION_EVENTS;

fn source_from_legacy(
    source_path: &str,
    bytes: &[u8],
) -> Result<AnimationEventListSource, AnimationEventListSourceError> {
    let mut source = AnimationEventListSource {
        source_path: normalize_source_path(source_path),
        animations: Vec::new(),
    };

    visit_animation_event_list(bytes, |item| {
        match item {
            AnimationEventItem::Animation(animation) => {
                source.animations.push(AnimationEventsSource {
                    animation: animation.name.into_owned(),
                    events: Vec::new(),
                });
            }
            AnimationEventItem::Event(event) => {
                let Some(animation) = source.animations.last_mut() else {
                    return Err(ParseError::EventWithoutAnimation);
                };
                animation.events.push(event_source(event));
            }
        }
        Ok(())
    })?;

    Ok(source)
}

fn event_source(event: AnimationEventRef<'_>) -> AnimationEventSource {
    AnimationEventSource {
        name: event.name.into_owned(),
        time: event.time,
        end_time: event.end_time,
        parameter: event.parameter.into_owned(),
        bone: event.bone.into_owned(),
        second_bone: event.second_bone.into_owned(),
        offset: AnimationEventVec3(event.offset.to_array()),
        direction: AnimationEventVec3(event.direction.to_array()),
        model: event.model.into_owned(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationEventListSourceTransform;

impl LegacySourceTransform for AnimationEventListSourceTransform {
    type Error = AnimationEventListSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_animation_event_list_source(&input.source_path) {
            return Err(AnimationEventListSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = source_from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            animation_event_list_source_path(&input.source_path),
            ANIMATION_EVENT_LIST_SOURCE_SCHEMA,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_animation_event_list_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".animevents")
}

#[must_use]
pub fn animation_event_list_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".animevents")
        .unwrap_or(&normalized);
    format!("{stem}.animevents.ron")
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventListSourceError {
    #[error("parse animation event list source: {0}")]
    Parse(#[from] ParseError),
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventListSourceTransformError {
    #[error("unsupported animation event list path {path}")]
    UnsupportedPath { path: String },
    #[error(transparent)]
    Source(#[from] AnimationEventListSourceError),
    #[error("serialize animation event list source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_emits_animation_event_list_ron_authoring_source() {
        let output = AnimationEventListSourceTransform
            .transform(LegacySourceInput::new("Animations/Events/Hero.animevents", br#"<anim_event_list>
 <animation name="animations/hero/idle.caf">
  <event name="footstep" time="0.25" endTime="0.5" parameter="FTSP" bone="Bip01" secondBone="Toe" offset="1,2,3" dir="0,1,0" model="objects/fx/foot.cgf"/>
 </animation>
</anim_event_list>"#))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("animation events should become authoring source");
        };
        assert_eq!(artifact.path, "animations/events/hero.animevents.ron");
        assert_eq!(artifact.schema, ANIMATION_EVENT_LIST_SOURCE_SCHEMA);

        let source = AnimationEventListSource::from_ron_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "animations/events/hero.animevents");
        assert_eq!(source.animations[0].animation, "animations/hero/idle.caf");
        let event = &source.animations[0].events[0];
        assert_eq!(event.name, "footstep");
        // Bit-exact: these round-trip through RON, so the values must come
        // back identical, not merely close.
        assert_eq!(event.time.to_bits(), 0.25_f32.to_bits());
        assert_eq!(event.end_time.to_bits(), 0.5_f32.to_bits());
        assert_eq!(event.parameter, "FTSP");
        assert_eq!(event.bone, "Bip01");
        assert_eq!(event.second_bone, "Toe");
        assert_eq!(
            event.offset.0.map(f32::to_bits),
            [1.0_f32, 2.0, 3.0].map(f32::to_bits)
        );
        assert_eq!(
            event.direction.0.map(f32::to_bits),
            [0.0_f32, 1.0, 0.0].map(f32::to_bits)
        );
        assert_eq!(event.model, "objects/fx/foot.cgf");

        let text = std::str::from_utf8(&artifact.bytes).unwrap();
        assert!(!text.contains("x: 1.0"));
        assert!(!text.contains("y: 2.0"));
        assert!(!text.contains("z: 3.0"));
    }
}
