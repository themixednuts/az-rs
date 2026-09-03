//! Named clocks, resolved into schedules (ADR 0041).
//!
//! A system does not choose `Update` or `FixedUpdate`; it names a clock, and
//! the composition resolves that name to a schedule and a rate. Nothing
//! defaults: an unresolvable name already failed at
//! [`Composer::finalize`](az_gem_contract::Composer::finalize), before this
//! module sees anything.
//!
//! Bevy has exactly one `Time<Fixed>` and one `FixedUpdate`, so a second fixed
//! clock cannot be a second write to that resource — it is a distinct schedule
//! driven by its own accumulator. That is what makes two clocks coexist on a
//! `unified` host instead of one silently overwriting the other.

use std::time::Duration;

use az_gem_contract::{ClockDefinition, ClockName, ClockUse, Registries, ResolvedClock};
use bevy::app::MainScheduleOrder;
use bevy::ecs::schedule::{ScheduleLabel, Schedules};
use bevy::prelude::*;

use crate::RuntimeAppError;

/// The schedule one composed clock resolves to.
///
/// A contribution schedules onto `Clock(SOME_CLOCK)` and declares the matching
/// [`ClockUse`], so the declaration and the placement are checked against each
/// other by [`audit`] rather than trusted.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Clock(pub ClockName);

/// The main-schedule slot that advances every composed clock.
///
/// Its own slot rather than a system inside `Update`, so where a clock's
/// systems run relative to the frame is stated once here instead of falling out
/// of ambiguous ordering.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Tick;

/// Longest span one frame may hand a fixed clock.
///
/// Bevy's own `Time<Fixed>` clamps at the same 250 ms, and for the same
/// reason: without a bound, one long frame schedules a catch-up burst that
/// makes the next frame longer still.
const CATCH_UP: Duration = Duration::from_millis(250);

struct Pace {
    clock: ClockName,
    /// `None` is the variable frame clock: one run per frame, no accumulator.
    period: Option<Duration>,
    carry: Duration,
}

impl Pace {
    fn steps(&mut self, delta: Duration) -> u32 {
        let Some(period) = self.period else {
            return 1;
        };
        self.carry = (self.carry + delta).min(CATCH_UP);
        let mut steps = 0;
        while self.carry >= period {
            self.carry -= period;
            steps += 1;
        }
        steps
    }
}

#[derive(Resource)]
struct Clocks(Vec<Pace>);

/// Give every composed clock a schedule and the accumulator that drives it.
///
/// # Errors
///
/// [`RuntimeAppError::InvalidClock`] when a definition carries a rate that is
/// not a positive, finite frequency — a clock nothing could tick.
pub fn install(app: &mut App, clocks: &[ResolvedClock]) -> Result<(), RuntimeAppError> {
    if clocks.is_empty() {
        return Ok(());
    }
    let mut paces = Vec::with_capacity(clocks.len());
    for clock in clocks {
        let period = match clock.rate_hz {
            None => None,
            Some(rate) if rate.is_finite() && rate > 0.0 => {
                Some(Duration::from_secs_f64(1.0 / rate))
            }
            Some(rate) => {
                return Err(RuntimeAppError::InvalidClock {
                    clock: clock.name,
                    rate,
                });
            }
        };
        app.init_schedule(Clock(clock.name));
        paces.push(Pace {
            clock: clock.name,
            period,
            carry: Duration::ZERO,
        });
    }
    app.insert_resource(Clocks(paces));
    app.init_schedule(Tick);
    app.world_mut()
        .resource_mut::<MainScheduleOrder>()
        .insert_after(Update, Tick);
    app.add_systems(Tick, advance);
    Ok(())
}

fn advance(world: &mut World) {
    let delta = world.resource::<Time>().delta();
    let mut plan = Vec::new();
    {
        let mut clocks = world.resource_mut::<Clocks>();
        for pace in &mut clocks.0 {
            plan.push((pace.clock, pace.steps(delta)));
        }
    }
    for (clock, steps) in plan {
        for _ in 0..steps {
            world.run_schedule(Clock(clock));
        }
    }
}

/// Assert every declared clock use is scheduled on that clock, and no other.
///
/// This ranges over the declaration set rather than over named systems, so it
/// fails whenever a declaration and a placement disagree — which is the only
/// way the mechanism can break. A system is matched by the stable label its
/// [`ClockUse`] carries appearing in the scheduled system's name — so this
/// needs Bevy's `debug` feature, which this workspace does not take by default.
/// A consumer that wants the assertion enables it, the way this crate's own
/// tests do.
///
/// # Errors
///
/// [`RuntimeAppError::ClockAbsent`] when a declared system is on no schedule
/// for its clock, [`RuntimeAppError::ClockStray`] when it also runs on another
/// composed clock, or [`RuntimeAppError::ClockSchedule`] when a clock's
/// schedule cannot be built.
pub fn audit(app: &mut App, registries: &Registries) -> Result<(), RuntimeAppError> {
    let uses = registries
        .get::<ClockUse>()
        .map(|registry| registry.entries().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    if uses.is_empty() {
        return Ok(());
    }
    let definitions = registries
        .get::<ClockDefinition>()
        .map(|registry| {
            registry
                .entries()
                .map(|definition| definition.name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut placed: Vec<(ClockName, Vec<String>)> = Vec::with_capacity(definitions.len());
    for clock in &definitions {
        // Taken out and put back the way Bevy runs a schedule: building one
        // touches world resources, including the schedule map itself.
        let Some(mut schedule) = app
            .world_mut()
            .resource_mut::<Schedules>()
            .remove(Clock(*clock))
        else {
            placed.push((*clock, Vec::new()));
            continue;
        };
        let built = schedule.initialize(app.world_mut());
        let names = schedule.systems().map_or_else(
            |_| Vec::new(),
            |systems| {
                systems
                    .map(|(_, system)| system.name().to_string())
                    .collect()
            },
        );
        app.world_mut().resource_mut::<Schedules>().insert(schedule);
        built.map_err(|error| RuntimeAppError::ClockSchedule {
            clock: *clock,
            detail: format!("{error:?}"),
        })?;
        placed.push((*clock, names));
    }

    for declared in uses {
        let carries = |clock: ClockName| {
            placed
                .iter()
                .find(|(candidate, _)| *candidate == clock)
                .is_some_and(|(_, names)| names.iter().any(|name| name.contains(declared.system)))
        };
        if !carries(declared.clock) {
            return Err(RuntimeAppError::ClockAbsent {
                clock: declared.clock,
                system: declared.system,
            });
        }
        if let Some((stray, _)) = placed
            .iter()
            .filter(|(clock, _)| *clock != declared.clock)
            .find(|(clock, _)| carries(*clock))
        {
            return Err(RuntimeAppError::ClockStray {
                clock: declared.clock,
                system: declared.system,
                stray: *stray,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pace(rate_hz: f64) -> Pace {
        Pace {
            clock: ClockName::new("test"),
            period: Some(Duration::from_secs_f64(1.0 / rate_hz)),
            carry: Duration::ZERO,
        }
    }

    #[test]
    fn a_fixed_clock_steps_its_own_rate_and_carries_the_remainder() {
        let mut clock = pace(100.0);

        assert_eq!(clock.steps(Duration::from_millis(100)), 10);
        assert_eq!(clock.steps(Duration::from_millis(5)), 0);
        assert_eq!(clock.steps(Duration::from_millis(5)), 1);
    }

    #[test]
    fn the_variable_clock_runs_once_a_frame_whatever_the_delta() {
        let mut clock = Pace {
            clock: ClockName::new("frame"),
            period: None,
            carry: Duration::ZERO,
        };

        assert_eq!(clock.steps(Duration::from_millis(1)), 1);
        assert_eq!(clock.steps(Duration::from_secs(1)), 1);
    }

    /// One long frame owes a bounded catch-up, not an unbounded burst that
    /// makes the next frame longer still.
    #[test]
    fn a_stalled_frame_cannot_owe_more_than_the_catch_up_bound() {
        let mut clock = pace(100.0);

        assert_eq!(clock.steps(Duration::from_secs(10)), 25);
        assert_eq!(clock.carry, Duration::ZERO);
    }
}
