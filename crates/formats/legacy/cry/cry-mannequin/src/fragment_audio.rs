//! Typed access to Mannequin fragment audio.
//!
//! Creature vocal/action sounds (bite, hurt, tail whip, body falls) are authored
//! as `ProcLayer` `type="Audio"` procedural clips inside Mannequin fragments —
//! not as `CryAnimation` animevents. Each clip names an ATL `StartTrigger`, an
//! optional `StopTrigger`, the `AttachmentJoint` the sound rides, and inherits
//! the owning `ProcLayer` `<Blend StartTime>` timing.
//!
//! A single logical fragment (e.g. `r_Death`) is assembled from several ADBs:
//! the animation database contributes the `AnimLayer` animation while a sibling
//! audio database contributes the `ProcLayer` audio for the same fragment name.
//! Callers merge [`MannequinFragmentAudio`] across databases by fragment name to
//! pair each animation with the sounds it fires.

use serde::{Deserialize, Serialize};

use crate::source_transform::{MannequinAnimationDatabase, MannequinFragment, MannequinProcedural};

/// The Mannequin procedural `type` that fires an ATL audio trigger.
pub const AUDIO_PROCEDURAL_TYPE: &str = "Audio";

/// Sentinel `StopTrigger` value meaning "do not stop" — never a real ATL trigger,
/// so it is dropped from [`MannequinAudioClip::stop_trigger`].
pub const DO_NOTHING_TRIGGER: &str = "do_nothing";

const START_TRIGGER_PARAM: &str = "StartTrigger";
const STOP_TRIGGER_PARAM: &str = "StopTrigger";
const ATTACHMENT_JOINT_PARAM: &str = "AttachmentJoint";

/// One `ProcLayer` `type="Audio"` clip: the ATL triggers it fires, the joint it
/// attaches to, and the layer-blend start time (seconds, relative to the
/// fragment's animation start).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MannequinAudioClip {
    /// ATL `StartTrigger` control name (e.g. `play_vox_alligator_hurt`).
    pub trigger: String,
    /// ATL `StopTrigger`, when it is a real trigger (the `do_nothing` sentinel is
    /// dropped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_trigger: Option<String>,
    /// Skeleton joint the sound is attached to (e.g. `bind_mouth_web`).
    pub joint: String,
    /// The owning `ProcLayer` `<Blend StartTime>` in seconds, when authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<f32>,
}

/// The `AnimLayer` animations and `ProcLayer` audio clips of one Mannequin fragment
/// group, gathered for pairing across animation/audio databases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MannequinFragmentAudio {
    /// Fragment group name (e.g. `Attack_Bite`, `r_Death`).
    pub fragment: String,
    /// The first non-empty fragment `Tags` string, when authored.
    pub tags: String,
    /// `AnimLayer` `<Animation name>` entries this fragment plays.
    pub animations: Vec<String>,
    /// `ProcLayer` `type="Audio"` clips this fragment fires.
    pub clips: Vec<MannequinAudioClip>,
}

impl MannequinAnimationDatabase {
    /// Typed access to every fragment's `AnimLayer` animations paired with its
    /// `ProcLayer` audio clips.
    ///
    /// One entry is produced per `FragmentList` group that carries at least one
    /// animation or one audio clip. Fragments in a group are merged (a group may
    /// hold several tag variants), so a group that only supplies audio (a sibling
    /// audio database) still appears — its animation comes from the matching group
    /// in the animation database, resolved by the caller.
    #[must_use]
    pub fn fragment_audio(&self) -> Vec<MannequinFragmentAudio> {
        let mut out = Vec::new();
        for group in &self.fragment_groups {
            let mut tags = String::new();
            let mut animations = Vec::new();
            let mut clips = Vec::new();
            for fragment in &group.fragments {
                if tags.is_empty()
                    && let Some(fragment_tags) =
                        fragment.tags.as_deref().filter(|value| !value.is_empty())
                {
                    fragment_tags.clone_into(&mut tags);
                }
                collect_fragment_animations(fragment, &mut animations);
                collect_fragment_audio(fragment, &mut clips);
            }
            if animations.is_empty() && clips.is_empty() {
                continue;
            }
            out.push(MannequinFragmentAudio {
                fragment: group.name.clone(),
                tags,
                animations,
                clips,
            });
        }
        out
    }
}

fn collect_fragment_animations(fragment: &MannequinFragment, out: &mut Vec<String>) {
    for layer in &fragment.animation_layers {
        for animation in &layer.animations {
            let name = animation.name.trim();
            if !name.is_empty()
                && !out
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(name))
            {
                out.push(name.to_owned());
            }
        }
    }
}

fn collect_fragment_audio(fragment: &MannequinFragment, out: &mut Vec<MannequinAudioClip>) {
    for layer in &fragment.procedural_layers {
        // The XML alternates `<Blend/>` then `<Procedural/>`, so the procedural at
        // index `i` inherits the blend at index `i`.
        for (index, procedural) in layer.procedurals.iter().enumerate() {
            if !procedural.ty.eq_ignore_ascii_case(AUDIO_PROCEDURAL_TYPE) {
                continue;
            }
            let Some(trigger) = param(procedural, START_TRIGGER_PARAM)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let stop_trigger = param(procedural, STOP_TRIGGER_PARAM)
                .map(|value| value.trim().to_owned())
                .filter(|value| {
                    !value.is_empty() && !value.eq_ignore_ascii_case(DO_NOTHING_TRIGGER)
                });
            let joint = param(procedural, ATTACHMENT_JOINT_PARAM)
                .map(|value| value.trim().to_owned())
                .unwrap_or_default();
            let start_time = layer.blends.get(index).and_then(|blend| blend.start_time);
            out.push(MannequinAudioClip {
                trigger,
                stop_trigger,
                joint,
                start_time,
            });
        }
    }
}

fn param(procedural: &MannequinProcedural, name: &str) -> Option<String> {
    procedural
        .parameters
        .iter()
        .find(|parameter| parameter.name.eq_ignore_ascii_case(name))
        .and_then(|parameter| parameter.value.clone())
}

#[cfg(test)]
mod tests {
    use crate::source_transform::MannequinAnimationDatabaseSource;

    const SAMPLE: &[u8] = br#"
        <AnimDB FragDef="actions.xml" TagDef="tags.xml">
          <FragmentList>
            <r_R0_B>
              <Fragment BlendOutDuration="0.2" Tags="">
                <AnimLayer>
                  <Blend ExitTime="0" StartTime="0" Duration="0"/>
                  <Animation name="alligator_r_r0_b"/>
                </AnimLayer>
                <ProcLayer>
                  <Blend ExitTime="0" StartTime="0.25" Duration="0"/>
                  <Procedural type="Audio" contextType="AudioContext">
                    <ProceduralParams>
                      <StartTrigger value="play_vox_alligator_hurt"/>
                      <StopTrigger value="do_nothing"/>
                      <AttachmentJoint value="bind_mouth_web"/>
                    </ProceduralParams>
                  </Procedural>
                </ProcLayer>
                <ProcLayer>
                  <Blend ExitTime="0" StartTime="0" Duration="0"/>
                  <Procedural type="Audio" contextType="AudioContext">
                    <ProceduralParams>
                      <StartTrigger value="play_sfx_aligator_tail_whip_fast"/>
                      <StopTrigger value="do_nothing"/>
                      <AttachmentJoint value="bind_tail_05"/>
                    </ProceduralParams>
                  </Procedural>
                </ProcLayer>
              </Fragment>
            </r_R0_B>
            <r_Death>
              <Fragment BlendOutDuration="0.2" Tags="">
                <ProcLayer>
                  <Blend ExitTime="0" StartTime="0" Duration="0"/>
                  <Procedural type="Audio" contextType="AudioContext">
                    <ProceduralParams>
                      <StartTrigger value="play_bodyfall_big"/>
                      <StopTrigger value="do_nothing"/>
                      <AttachmentJoint value="bind_pelvis"/>
                    </ProceduralParams>
                  </Procedural>
                </ProcLayer>
              </Fragment>
            </r_Death>
            <Idle>
              <Fragment BlendOutDuration="0.2" Tags="">
                <AnimLayer>
                  <Blend ExitTime="0" StartTime="0" Duration="0.4"/>
                  <Animation name="alligator_idle" flags="Loop"/>
                </AnimLayer>
              </Fragment>
            </Idle>
          </FragmentList>
        </AnimDB>
    "#;

    #[test]
    fn extracts_audio_clips_and_animations_per_fragment() {
        let database = MannequinAnimationDatabaseSource::from_legacy("test.adb", SAMPLE)
            .unwrap()
            .database;
        let fragments = database.fragment_audio();

        let bite = fragments.iter().find(|f| f.fragment == "r_R0_B").unwrap();
        assert_eq!(bite.animations, ["alligator_r_r0_b"]);
        assert_eq!(bite.clips.len(), 2);
        assert_eq!(bite.clips[0].trigger, "play_vox_alligator_hurt");
        // The `do_nothing` sentinel StopTrigger is dropped.
        assert_eq!(bite.clips[0].stop_trigger, None);
        assert_eq!(bite.clips[0].joint, "bind_mouth_web");
        assert_eq!(bite.clips[0].start_time, Some(0.25));
        assert_eq!(bite.clips[1].trigger, "play_sfx_aligator_tail_whip_fast");
        assert_eq!(bite.clips[1].joint, "bind_tail_05");

        // An audio-only fragment (no AnimLayer) still surfaces so the animation
        // database's matching fragment can supply the animation by name.
        let death = fragments.iter().find(|f| f.fragment == "r_Death").unwrap();
        assert!(death.animations.is_empty());
        assert_eq!(death.clips.len(), 1);
        assert_eq!(death.clips[0].trigger, "play_bodyfall_big");

        // A pure-animation fragment surfaces too (with no clips): the merge keys
        // on fragment name, so the animation must be visible even when the audio
        // lives in a sibling database.
        let idle = fragments.iter().find(|f| f.fragment == "Idle").unwrap();
        assert_eq!(idle.animations, ["alligator_idle"]);
        assert!(idle.clips.is_empty());
    }
}
