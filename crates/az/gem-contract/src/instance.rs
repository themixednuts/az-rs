use std::fmt;

use crate::ids::{ContributionId, GemId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId {
    pub gem: GemId,
    pub contribution: ContributionId,
    pub generation: u32,
}

impl InstanceId {
    #[must_use]
    pub const fn new(gem: GemId, contribution: ContributionId, generation: u32) -> Self {
        Self {
            gem,
            contribution,
            generation,
        }
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}#{}",
            self.gem, self.contribution, self.generation
        )
    }
}
