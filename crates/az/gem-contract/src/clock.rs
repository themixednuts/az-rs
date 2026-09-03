use crate::capability::Unconditional;
use crate::ids::ClockName;
use crate::registry::RegistryEntry;

/// A named clock made available by this composition.
///
/// Duplicate names are a compose-time error like any other registry key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockDefinition {
    pub name: ClockName,
    /// Fixed rate in Hz; `None` is the variable frame clock — the ordinary
    /// case is explicit, not inferred from absence (ADR 0041).
    pub rate_hz: Option<f64>,
}

impl RegistryEntry for ClockDefinition {
    type Key = ClockName;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "clock-definition"
    }

    fn key(&self) -> ClockName {
        self.name
    }
}

/// One system's declared clock.
///
/// Finalize errors on any use whose clock has no composed
/// [`ClockDefinition`]; nothing ever defaults to a schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockUse {
    pub clock: ClockName,
    /// Stable system label; unique within the composition so the
    /// schedule-inspecting harness can attribute placements.
    pub system: &'static str,
}

impl RegistryEntry for ClockUse {
    type Key = String;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "clock-use"
    }

    fn key(&self) -> String {
        format!("{}::{}", self.clock, self.system)
    }
}
