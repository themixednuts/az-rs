use thiserror::Error;

use crate::ids::ClockName;
use crate::instance::InstanceId;
use crate::role::GemTargetRole;

/// Composition failure.
///
/// Duplicates name both culprits (ADR 0033); an unresolvable clock is a
/// composition error, never a default (ADR 0041).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ComposeError {
    #[error("duplicate {registry} entry `{key}`: first registered by {first}, again by {second}")]
    Duplicate {
        registry: &'static str,
        key: String,
        first: InstanceId,
        second: InstanceId,
    },

    #[error(
        "clock `{clock}` declared by {instance} does not resolve on host `{role}`: no clock definition composed"
    )]
    UnresolvableClock {
        clock: ClockName,
        instance: InstanceId,
        role: GemTargetRole,
    },
}
