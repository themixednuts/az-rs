//! Timestamped, acknowledged camera-drag transitions for the producer thread.

use std::{collections::VecDeque, time::Instant};

use az_editor_ui::panels::ViewportCameraDragKind;
use bevy::prelude::Vec2;

#[derive(Clone, Copy, Debug)]
pub enum CameraDragTransition {
    Start {
        interaction_id: u64,
        kind: ViewportCameraDragKind,
        position: Vec2,
        at: Instant,
    },
    End {
        interaction_id: u64,
        position: Vec2,
        at: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraDragAcknowledged {
    pub interaction_id: u64,
    pub production_frame: u64,
    pub started_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDragSample {
    pub interaction_id: u64,
    pub kind: ViewportCameraDragKind,
    pub delta: Vec2,
    pub sampled_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraDragFinished {
    pub interaction_id: u64,
    pub production_frame: u64,
    pub ended_at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct ActiveCameraDrag {
    interaction_id: u64,
    kind: ViewportCameraDragKind,
    last_position: Vec2,
    acknowledged_frame: u64,
}

/// Ordered semantic transitions plus latest absolute samples. A matching end
/// cannot be consumed on the frame that acknowledged its start.
#[derive(Debug, Default)]
pub struct CameraDragTimeline {
    transitions: VecDeque<CameraDragTransition>,
    active: Option<ActiveCameraDrag>,
    orphaned_end_count: u64,
}

impl CameraDragTimeline {
    pub(crate) fn push(&mut self, transition: CameraDragTransition) {
        self.transitions.push_back(transition);
    }

    pub(crate) fn acknowledge_next(
        &mut self,
        production_frame: u64,
    ) -> Option<CameraDragAcknowledged> {
        if self.active.is_some() {
            return None;
        }
        while matches!(
            self.transitions.front(),
            Some(CameraDragTransition::End { .. })
        ) {
            self.transitions.pop_front();
            self.orphaned_end_count += 1;
        }
        let CameraDragTransition::Start {
            interaction_id,
            kind,
            position,
            at,
        } = self.transitions.pop_front()?
        else {
            unreachable!("orphaned ends were removed above")
        };
        self.active = Some(ActiveCameraDrag {
            interaction_id,
            kind,
            last_position: position,
            acknowledged_frame: production_frame,
        });
        Some(CameraDragAcknowledged {
            interaction_id,
            production_frame,
            started_at: at,
        })
    }

    pub(crate) fn sample(&mut self, fresh: Option<(Vec2, Instant)>) -> Option<CameraDragSample> {
        let active = self.active.as_mut()?;
        let endpoint = self
            .transitions
            .iter()
            .find_map(|transition| match transition {
                CameraDragTransition::End {
                    interaction_id,
                    position,
                    at,
                } if *interaction_id == active.interaction_id => Some((*position, *at)),
                _ => None,
            });
        let (position, sampled_at) = endpoint.or(fresh)?;
        let delta = position - active.last_position;
        active.last_position = position;
        Some(CameraDragSample {
            interaction_id: active.interaction_id,
            kind: active.kind,
            delta,
            sampled_at,
        })
    }

    pub(crate) fn finish_acknowledged(
        &mut self,
        production_frame: u64,
    ) -> Option<CameraDragFinished> {
        let active = self.active?;
        if production_frame <= active.acknowledged_frame {
            return None;
        }
        let index = self.transitions.iter().position(|transition| {
            matches!(transition, CameraDragTransition::End { interaction_id, .. }
                if *interaction_id == active.interaction_id)
        })?;
        let CameraDragTransition::End {
            interaction_id, at, ..
        } = self
            .transitions
            .remove(index)
            .expect("transition index came from this queue")
        else {
            unreachable!("matched transition is an end")
        };
        self.active = None;
        Some(CameraDragFinished {
            interaction_id,
            production_frame,
            ended_at: at,
        })
    }

    #[must_use]
    pub(crate) const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    #[must_use]
    pub(crate) fn take_orphaned_end_count(&mut self) -> u64 {
        std::mem::take(&mut self.orphaned_end_count)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn start_and_end_in_one_batch_still_apply_full_motion() {
        let now = Instant::now();
        let mut timeline = CameraDragTimeline::default();
        timeline.push(CameraDragTransition::Start {
            interaction_id: 7,
            kind: ViewportCameraDragKind::Orbit,
            position: Vec2::new(0.2, 0.3),
            at: now,
        });
        timeline.push(CameraDragTransition::End {
            interaction_id: 7,
            position: Vec2::new(0.8, 0.6),
            at: now + Duration::from_millis(20),
        });

        let acknowledged = timeline.acknowledge_next(10).unwrap();
        assert_eq!(acknowledged.interaction_id, 7);
        let sample = timeline.sample(None).unwrap();
        assert_eq!(sample.delta, Vec2::new(0.6, 0.3));
        assert!(timeline.finish_acknowledged(10).is_none());
        assert!(timeline.finish_acknowledged(11).is_some());
    }

    #[test]
    fn repeated_gestures_each_receive_their_own_boundary() {
        let now = Instant::now();
        let mut timeline = CameraDragTimeline::default();
        for id in 1..=2 {
            timeline.push(CameraDragTransition::Start {
                interaction_id: id,
                kind: ViewportCameraDragKind::Orbit,
                position: Vec2::ZERO,
                at: now,
            });
            timeline.push(CameraDragTransition::End {
                interaction_id: id,
                position: Vec2::ONE,
                at: now,
            });
        }

        assert_eq!(timeline.acknowledge_next(1).unwrap().interaction_id, 1);
        assert!(timeline.finish_acknowledged(1).is_none());
        assert_eq!(timeline.finish_acknowledged(2).unwrap().interaction_id, 1);
        assert_eq!(timeline.acknowledge_next(3).unwrap().interaction_id, 2);
        assert_eq!(timeline.finish_acknowledged(4).unwrap().interaction_id, 2);
        assert_eq!(timeline.take_orphaned_end_count(), 0);
    }

    #[test]
    fn fresh_absolute_positions_are_latest_wins_while_active() {
        let now = Instant::now();
        let mut timeline = CameraDragTimeline::default();
        timeline.push(CameraDragTransition::Start {
            interaction_id: 1,
            kind: ViewportCameraDragKind::Pan,
            position: Vec2::new(0.1, 0.1),
            at: now,
        });
        timeline.acknowledge_next(1);
        let sample = timeline
            .sample(Some((Vec2::new(0.4, 0.5), now + Duration::from_millis(1))))
            .unwrap();
        assert_eq!(sample.delta, Vec2::new(0.3, 0.4));
    }
}
