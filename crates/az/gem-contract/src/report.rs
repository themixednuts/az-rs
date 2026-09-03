use std::fmt;

use crate::capability::{CapSet, HostCapability, Refusal};
use crate::ids::ClockName;
use crate::instance::InstanceId;
use crate::role::GemTargetRole;

/// One attributed registration, as it appears in the compose report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub instance: InstanceId,
    pub registry: &'static str,
    pub key: String,
}

/// A `when` narrowing that did not run on this host.
///
/// Visible, never silent (ADR 0041).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclinedNarrowing {
    pub instance: InstanceId,
    pub capability: HostCapability,
}

/// A named clock this composition resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedClock {
    pub name: ClockName,
    /// Fixed rate in Hz; `None` is the variable frame clock.
    pub rate_hz: Option<f64>,
}

/// The operator-visible composition record.
///
/// ADR 0033, amended by 0041 with the raw-access and declined-narrowing
/// sections.
#[derive(Clone, Debug, PartialEq)]
pub struct ComposeReport {
    pub role: GemTargetRole,
    pub provided: CapSet,
    pub composed: Vec<InstanceId>,
    pub entries: Vec<Registration>,
    pub clocks: Vec<ResolvedClock>,
    /// Contributions that took the raw `&mut App` hatch.
    ///
    /// Not gated on the `app` feature: report shape is one shape everywhere,
    /// so operators and tooling read the same struct either way. Without the
    /// feature there is no hatch to take, so this is always empty.
    pub raw_app_access: Vec<InstanceId>,
    pub declined_narrowings: Vec<DeclinedNarrowing>,
    pub refusals: Vec<Refusal>,
}

impl fmt::Display for ComposeReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "== compose report: host `{}` provides {} ==",
            self.role, self.provided
        )?;
        writeln!(formatter, "  composed ({}):", self.composed.len())?;
        for instance in &self.composed {
            writeln!(formatter, "    {instance}")?;
        }
        writeln!(formatter, "  entries ({}):", self.entries.len())?;
        for entry in &self.entries {
            writeln!(
                formatter,
                "    [{}] {} -> {}",
                entry.registry, entry.instance, entry.key
            )?;
        }
        writeln!(formatter, "  clocks ({}):", self.clocks.len())?;
        for clock in &self.clocks {
            match clock.rate_hz {
                Some(rate) => writeln!(formatter, "    {} @ {rate} Hz", clock.name)?,
                None => writeln!(formatter, "    {} (variable frame clock)", clock.name)?,
            }
        }
        writeln!(
            formatter,
            "  raw &mut App access ({}):",
            self.raw_app_access.len()
        )?;
        for instance in &self.raw_app_access {
            writeln!(formatter, "    {instance}")?;
        }
        writeln!(
            formatter,
            "  declined narrowings ({}):",
            self.declined_narrowings.len()
        )?;
        for declined in &self.declined_narrowings {
            writeln!(
                formatter,
                "    {} when<{}>",
                declined.instance, declined.capability
            )?;
        }
        writeln!(formatter, "  refusals ({}):", self.refusals.len())?;
        for refusal in &self.refusals {
            writeln!(formatter, "    {refusal}")?;
        }
        Ok(())
    }
}
