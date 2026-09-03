//! Sequencer projection and transport controller.
//!
//! v1 projects the selected animation motion's event tracks into the
//! editor-ui [`EditorSequenceTimeline`] global. The timer advances only this
//! transport projection; authored `.seq` document loading and key editing are
//! reserved for the project-host sequence schema seam.

use std::time::{Duration, Instant};

use az_editor_ui::actions::{SeqGoToKey, SeqKeyTarget, SeqPlay, SeqScrub, SeqSetLoop, SeqStop};
use az_editor_ui::panels::modes::sequencer::EditorSequenceTimeline;
use az_editor_ui::panels::{EditorAnimationPreviewCatalog, EditorMannequinPreview};
use gpui::App;
use tracing::{info, instrument};

const PLAYBACK_TICK: Duration = Duration::from_millis(16);
const SOURCE_SYNC_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceTransportAction {
    SetPlaying(bool),
    Stop,
    Scrub(u32),
    SetLooping(bool),
    GoTo(SeqKeyTarget),
    Advance(u32),
}

/// Apply one pure transport transition to the published timeline projection.
pub fn apply_sequence_transport(
    timeline: &mut EditorSequenceTimeline,
    action: SequenceTransportAction,
) -> bool {
    let before = timeline.clone();
    match action {
        SequenceTransportAction::SetPlaying(playing) => {
            timeline.playing = playing && timeline.has_source();
            if timeline.playing && timeline.position_millis >= timeline.duration_millis {
                timeline.position_millis = 0;
            }
        }
        SequenceTransportAction::Stop => {
            timeline.playing = false;
            timeline.position_millis = 0;
        }
        SequenceTransportAction::Scrub(position_millis) => {
            timeline.position_millis = position_millis.min(timeline.duration_millis);
        }
        SequenceTransportAction::SetLooping(looping) => {
            timeline.looping = looping;
        }
        SequenceTransportAction::GoTo(target) => {
            let keys = timeline.key_positions();
            timeline.position_millis = match target {
                SeqKeyTarget::Start => 0,
                SeqKeyTarget::Previous => keys
                    .into_iter()
                    .rev()
                    .find(|position| *position < timeline.position_millis)
                    .unwrap_or(0),
                SeqKeyTarget::Next => keys
                    .into_iter()
                    .find(|position| *position > timeline.position_millis)
                    .unwrap_or(timeline.duration_millis),
                SeqKeyTarget::End => timeline.duration_millis,
            };
        }
        SequenceTransportAction::Advance(elapsed_millis) => {
            if timeline.playing && timeline.has_source() && elapsed_millis > 0 {
                let next = timeline.position_millis.saturating_add(elapsed_millis);
                if next >= timeline.duration_millis {
                    if timeline.looping {
                        timeline.position_millis = next % timeline.duration_millis;
                    } else {
                        timeline.position_millis = timeline.duration_millis;
                        timeline.playing = false;
                    }
                } else {
                    timeline.position_millis = next;
                }
            }
        }
    }
    *timeline != before
}

/// Retains the cancellation sender for the attached sequencer timer.
pub(crate) struct EditorSequencerController {
    _close: tokio::sync::watch::Sender<()>,
}

/// Install typed action handlers and publish the initial empty projection.
#[instrument(skip(cx))]
pub fn install_sequencer_action_handlers(cx: &mut App) {
    cx.set_global(EditorSequenceTimeline::empty());

    cx.on_action(|action: &SeqPlay, cx| {
        update_sequence_transport(cx, SequenceTransportAction::SetPlaying(action.playing));
    });
    cx.on_action(|_: &SeqStop, cx| {
        update_sequence_transport(cx, SequenceTransportAction::Stop);
    });
    cx.on_action(|action: &SeqScrub, cx| {
        update_sequence_transport(cx, SequenceTransportAction::Scrub(action.position_millis));
    });
    cx.on_action(|action: &SeqSetLoop, cx| {
        update_sequence_transport(cx, SequenceTransportAction::SetLooping(action.looping));
    });
    cx.on_action(|action: &SeqGoToKey, cx| {
        update_sequence_transport(cx, SequenceTransportAction::GoTo(action.target));
    });

    info!("installed sequencer action handlers");
}

/// Start the GPUI playback timer and selected-motion projection watch.
#[instrument(skip(cx, _session))]
pub(crate) fn install_sequencer_slot(
    cx: &mut App,
    _session: crate::EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let (close, mut close_rx) = tokio::sync::watch::channel(());
    if !crate::controller_set::complete_sequencer(
        cx,
        fence,
        EditorSequencerController { _close: close },
    ) {
        return;
    }
    if sync_sequence_source(cx) {
        cx.refresh_windows();
    }

    cx.spawn(async move |cx| {
        let mut last_tick = Instant::now();
        let mut since_source_sync = SOURCE_SYNC_INTERVAL;
        loop {
            tokio::select! {
                _ = close_rx.changed() => break,
                () = cx.background_executor().timer(PLAYBACK_TICK) => {}
            }
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(last_tick);
            last_tick = now;
            since_source_sync = since_source_sync.saturating_add(elapsed);
            let should_sync_source = since_source_sync >= SOURCE_SYNC_INTERVAL;
            if should_sync_source {
                since_source_sync = Duration::ZERO;
            }
            // The `min` above already bounds the value, so the fallback is
            // unreachable and only keeps the narrowing checked.
            let elapsed_millis =
                u32::try_from(elapsed.as_millis().min(u128::from(u32::MAX))).unwrap_or(u32::MAX);
            if !cx.update(move |cx| {
                pump_sequence_timeline(cx, fence, elapsed_millis, should_sync_source)
            }) {
                break;
            }
        }
    })
    .detach();

    info!("installed sequencer selected-motion watch and playback timer");
}

fn update_sequence_transport(cx: &mut App, action: SequenceTransportAction) -> bool {
    let source_changed = sync_sequence_source(cx);
    let transport_changed = {
        let timeline = cx.default_global::<EditorSequenceTimeline>();
        apply_sequence_transport(timeline, action)
    };
    if source_changed || transport_changed {
        cx.refresh_windows();
    }
    source_changed || transport_changed
}

fn pump_sequence_timeline(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    elapsed_millis: u32,
    should_sync_source: bool,
) -> bool {
    if !crate::controller_set::is_current_fence(cx, fence) {
        return false;
    }

    let source_changed = should_sync_source && sync_sequence_source(cx);
    let transport_changed = {
        let timeline = cx.default_global::<EditorSequenceTimeline>();
        apply_sequence_transport(timeline, SequenceTransportAction::Advance(elapsed_millis))
    };
    if source_changed || transport_changed {
        cx.refresh_windows();
    }
    true
}

fn sync_sequence_source(cx: &mut App) -> bool {
    let motion = {
        let preview = cx.try_global::<EditorMannequinPreview>();
        cx.try_global::<EditorAnimationPreviewCatalog>()
            .and_then(|catalog| catalog.selected_motion(preview))
            .cloned()
    };
    let current = cx
        .try_global::<EditorSequenceTimeline>()
        .cloned()
        .unwrap_or_default();
    let next = match motion {
        Some(motion) => {
            let same_source = current.source_motion_path.as_deref() == Some(&motion.asset_path);
            EditorSequenceTimeline::from_motion(
                &motion,
                if same_source {
                    current.position_millis
                } else {
                    0
                },
                same_source && current.playing,
                current.looping,
            )
        }
        None => EditorSequenceTimeline::empty(),
    };
    if next == current {
        return false;
    }
    cx.set_global(next);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{EditorAnimationEventData, EditorAnimationMotionData};

    fn timeline() -> EditorSequenceTimeline {
        EditorSequenceTimeline::from_motion(
            &EditorAnimationMotionData {
                asset_path: "animations/test.anim.glb".to_owned(),
                name: "test".to_owned(),
                set_path: "animations".to_owned(),
                duration_millis: Some(1_000),
                channel_count: 1,
                joint_targets: Vec::new(),
                events: vec![EditorAnimationEventData {
                    name: "impact".to_owned(),
                    animation: "test".to_owned(),
                    time_millis: 400,
                    end_time_millis: 600,
                    parameter: "heavy".to_owned(),
                }],
                pipeline_status: None,
            },
            0,
            false,
            false,
        )
    }

    #[test]
    fn transport_transitions_play_stop_and_finish_or_loop() {
        let mut timeline = timeline();

        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::SetPlaying(true)
        ));
        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::Advance(1_250)
        ));
        assert_eq!((timeline.position_millis, timeline.playing), (1_000, false));

        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::SetLooping(true)
        ));
        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::SetPlaying(true)
        ));
        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::Advance(1_250)
        ));
        assert_eq!((timeline.position_millis, timeline.playing), (250, true));

        assert!(apply_sequence_transport(
            &mut timeline,
            SequenceTransportAction::Stop
        ));
        assert_eq!((timeline.position_millis, timeline.playing), (0, false));
    }

    #[test]
    fn dropping_the_slot_owned_controller_cancels_the_playback_timer() {
        let (close, mut close_rx) = tokio::sync::watch::channel(());
        let controller = EditorSequencerController { _close: close };

        drop(controller);

        assert!(
            futures::executor::block_on(close_rx.changed()).is_err(),
            "the timer receiver must observe aggregate-slot replacement"
        );
    }
}
