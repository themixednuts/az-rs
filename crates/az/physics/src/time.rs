use bevy_ecs::resource::Resource;
use bevy_reflect::Reflect;

use crate::PhysicsError;

const SUBSTEP_COUNT_ADJUSTMENT_FACTOR: f32 = 0.05;
const VARIABLE_STEP_SMOOTHING_FACTOR: f32 = 0.1;
const ADAPTIVE_STEP_STABILITY_FRAMES: u32 = 5;

/// World stepping values exposed by `Physics::WorldConfiguration`.
///
/// The default `RockNRoll` world uses a 50 ms maximum, no fixed timestep, and
/// a two-substep cap. A positive fixed timestep selects the accumulator path;
/// zero selects one clamped variable step.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Reflect)]
pub struct PhysicsStepConfiguration {
    pub maximum_time_step: f32,
    pub fixed_time_step: f32,
    /// Zero means unlimited, matching the native selector.
    pub maximum_sub_steps: u32,
    pub paused: bool,
}

impl Default for PhysicsStepConfiguration {
    fn default() -> Self {
        Self {
            maximum_time_step: 0.05,
            fixed_time_step: 0.0,
            maximum_sub_steps: 2,
            paused: false,
        }
    }
}

impl PhysicsStepConfiguration {
    #[inline]
    #[must_use]
    pub const fn mode(self) -> PhysicsTimeStepMode {
        if self.fixed_time_step > 0.0 {
            PhysicsTimeStepMode::Fixed
        } else {
            PhysicsTimeStepMode::Variable
        }
    }
}

/// Behavioral names for the four `RockNRoll` time-step modes.
///
/// `Fixed` and `Variable` follow the world-wrapper branch. The other two names
/// describe their behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[repr(u8)]
pub enum PhysicsTimeStepMode {
    Fixed = 0,
    AdaptiveFixed = 1,
    Variable = 2,
    SmoothedVariable = 3,
}

/// Statistics for the most recently requested world update.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Reflect)]
pub struct PhysicsStepReport {
    pub requested_time_step: f32,
    pub simulated_time: f32,
    pub substep_count: u32,
}

/// Stateful `RockNRoll` timestep selector.
///
/// This is deliberately independent of any solver. It yields borrowed
/// substep durations and leaves each backend responsible only for advancing
/// its scene by those durations.
#[derive(Resource, Debug, Clone)]
pub struct PhysicsTimeStepSelector {
    configuration: PhysicsStepConfiguration,
    mode: PhysicsTimeStepMode,
    step_count: u32,
    previous_step_count: u32,
    accumulated_time: f32,
    count_adjustment_remaining: f32,
    variable_step: f32,
    fixed_step: f32,
    adaptive_step_count: u32,
    candidate_step_count: u32,
    candidate_stability_frames: u32,
    exhausted: bool,
}

impl Default for PhysicsTimeStepSelector {
    fn default() -> Self {
        Self::new(PhysicsStepConfiguration::default())
    }
}

impl PhysicsTimeStepSelector {
    #[must_use]
    pub const fn new(configuration: PhysicsStepConfiguration) -> Self {
        Self::with_mode(configuration, configuration.mode())
    }

    /// Construct one of the two internal selector modes not exposed through
    /// `Physics::WorldConfiguration`.
    #[must_use]
    pub const fn with_mode(
        configuration: PhysicsStepConfiguration,
        mode: PhysicsTimeStepMode,
    ) -> Self {
        Self {
            configuration,
            mode,
            step_count: 0,
            previous_step_count: 0,
            accumulated_time: 0.0,
            count_adjustment_remaining: 0.0,
            variable_step: 0.0,
            fixed_step: configuration.fixed_time_step,
            adaptive_step_count: 1,
            candidate_step_count: 1,
            candidate_stability_frames: 0,
            exhausted: true,
        }
    }

    #[inline]
    #[must_use]
    pub const fn mode(&self) -> PhysicsTimeStepMode {
        self.mode
    }

    /// Rebuild the native selector when its world configuration changes.
    #[expect(
        clippy::float_cmp,
        reason = "this is authored-value change detection, not a physical measurement: any epsilon would keep a stale selector alive after a caller writes a nearby maximum or fixed step"
    )]
    pub fn reconfigure(&mut self, configuration: PhysicsStepConfiguration) {
        let selector_changed = self.configuration.maximum_time_step
            != configuration.maximum_time_step
            || self.configuration.fixed_time_step != configuration.fixed_time_step
            || self.configuration.maximum_sub_steps != configuration.maximum_sub_steps;
        if selector_changed || self.mode != configuration.mode() {
            *self = Self::new(configuration);
        } else {
            self.configuration.paused = configuration.paused;
        }
    }

    /// Prepare one frame and borrow its native substep sequence.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidTimeStep`] when `requested_time_step` is
    /// non-finite or not greater than zero. This is the only failure: once the
    /// frame is prepared, iteration cannot fail.
    pub fn substeps(
        &mut self,
        requested_time_step: f32,
    ) -> Result<PhysicsSubsteps<'_>, PhysicsError> {
        if !requested_time_step.is_finite() || requested_time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(requested_time_step));
        }

        self.prepare(requested_time_step);
        Ok(PhysicsSubsteps { selector: self })
    }

    fn prepare(&mut self, requested_time_step: f32) {
        match self.mode {
            PhysicsTimeStepMode::Fixed | PhysicsTimeStepMode::AdaptiveFixed => {
                self.previous_step_count = self.step_count;
                self.step_count = 0;
                self.accumulated_time += requested_time_step;

                if self.mode == PhysicsTimeStepMode::AdaptiveFixed {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the native selector floors the substep ratio; a negative or non-finite ratio saturates to zero and the following max(1) restores the native floor of one substep"
                    )]
                    let candidate =
                        ((requested_time_step / self.configuration.fixed_time_step) as u32).max(1);
                    if candidate != self.candidate_step_count {
                        self.candidate_step_count = candidate;
                        self.candidate_stability_frames = 0;
                        self.exhausted = false;
                        return;
                    }

                    self.candidate_stability_frames += 1;
                    if candidate != self.adaptive_step_count
                        && self.candidate_stability_frames >= ADAPTIVE_STEP_STABILITY_FRAMES
                    {
                        self.adaptive_step_count = candidate;
                        self.candidate_stability_frames = 0;
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "candidate is a per-frame substep count derived from one \
                                      frame's time step divided by the fixed step, so it stays \
                                      far below f32's 24-bit exact integer range"
                        )]
                        let steps = candidate as f32;
                        self.fixed_step = steps * self.configuration.fixed_time_step;
                    }
                }
                self.exhausted = false;
            }
            PhysicsTimeStepMode::Variable | PhysicsTimeStepMode::SmoothedVariable => {
                let clamped = if self.configuration.maximum_time_step > 0.0 {
                    requested_time_step.min(self.configuration.maximum_time_step)
                } else {
                    requested_time_step
                };
                self.exhausted = false;
                if self.mode == PhysicsTimeStepMode::SmoothedVariable {
                    self.variable_step = VARIABLE_STEP_SMOOTHING_FACTOR.mul_add(
                        clamped,
                        (1.0 - VARIABLE_STEP_SMOOTHING_FACTOR) * self.variable_step,
                    );
                } else {
                    self.variable_step = clamped;
                }
            }
        }
    }

    fn next_substep(&mut self) -> Option<f32> {
        if self.exhausted {
            return None;
        }
        if matches!(
            self.mode,
            PhysicsTimeStepMode::Variable | PhysicsTimeStepMode::SmoothedVariable
        ) {
            self.exhausted = true;
            return Some(self.variable_step);
        }

        let enough_accumulated_time = self.accumulated_time >= self.fixed_step;
        let preserve_previous_count =
            !enough_accumulated_time && self.step_count < self.previous_step_count;
        let suppress_new_count =
            enough_accumulated_time && self.previous_step_count <= self.step_count;
        let suppress_step = if (preserve_previous_count || suppress_new_count)
            && SUBSTEP_COUNT_ADJUSTMENT_FACTOR > 0.0
            && self.count_adjustment_remaining <= 0.0
        {
            self.count_adjustment_remaining = self.fixed_step / SUBSTEP_COUNT_ADJUSTMENT_FACTOR;
            if preserve_previous_count {
                self.accumulated_time = 0.0;
                return self.emit_fixed_substep();
            }
            true
        } else {
            false
        };

        if !enough_accumulated_time {
            self.exhausted = true;
            return None;
        }

        self.accumulated_time -= self.fixed_step;
        if suppress_step {
            self.exhausted = true;
            return None;
        }
        self.emit_fixed_substep()
    }

    fn emit_fixed_substep(&mut self) -> Option<f32> {
        if self.configuration.maximum_sub_steps != 0
            && self.step_count >= self.configuration.maximum_sub_steps
        {
            self.accumulated_time = 0.0;
            self.exhausted = true;
            return None;
        }

        self.step_count += 1;
        self.count_adjustment_remaining -= self.fixed_step;
        Some(self.fixed_step)
    }
}

/// Allocation-free iterator over one prepared frame's substeps.
pub struct PhysicsSubsteps<'a> {
    selector: &'a mut PhysicsTimeStepSelector,
}

impl Iterator for PhysicsSubsteps<'_> {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.selector.next_substep()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_variable_mode_emits_one_clamped_step() {
        let mut selector = PhysicsTimeStepSelector::default();
        assert_eq!(selector.substeps(0.1).unwrap().collect::<Vec<_>>(), [0.05]);
        assert_eq!(selector.substeps(0.02).unwrap().collect::<Vec<_>>(), [0.02]);
    }

    #[test]
    fn fixed_mode_accumulates_and_honors_native_substep_cap() {
        let configuration = PhysicsStepConfiguration {
            fixed_time_step: 0.02,
            maximum_sub_steps: 2,
            ..Default::default()
        };
        let mut selector = PhysicsTimeStepSelector::new(configuration);

        // The native count-stability gate suppresses the first count change.
        assert_eq!(selector.substeps(0.02).unwrap().count(), 0);
        assert_eq!(
            selector.substeps(0.06).unwrap().collect::<Vec<_>>(),
            [0.02, 0.02]
        );
    }

    #[test]
    fn smoothed_mode_uses_exponential_filter() {
        let configuration = PhysicsStepConfiguration::default();
        let mut selector = PhysicsTimeStepSelector::with_mode(
            configuration,
            PhysicsTimeStepMode::SmoothedVariable,
        );
        let first = selector.substeps(0.05).unwrap().next().unwrap();
        let second = selector.substeps(0.05).unwrap().next().unwrap();
        assert!((first - 0.005).abs() < 1.0e-6);
        assert!((second - 0.0095).abs() < 1.0e-6);
    }

    #[test]
    fn adaptive_mode_adopts_a_step_count_after_five_stable_frames() {
        let configuration = PhysicsStepConfiguration {
            fixed_time_step: 0.01,
            maximum_sub_steps: 0,
            ..Default::default()
        };
        let mut selector =
            PhysicsTimeStepSelector::with_mode(configuration, PhysicsTimeStepMode::AdaptiveFixed);
        // The first observation changes the candidate; five subsequent
        // matching frames satisfy the native stability counter.
        for _ in 0..6 {
            let _ = selector.substeps(0.03).unwrap().count();
        }
        assert_eq!(selector.adaptive_step_count, 3);
        assert!((selector.fixed_step - 0.03).abs() < 1.0e-6);
    }
}
