use crate::ids::{CapabilityId, ContributionId, GemId};
use crate::role::GemTargetRole;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContributionDescriptor {
    pub gem: GemId,
    pub contribution: ContributionId,
    pub roles: &'static [GemTargetRole],
}

impl ContributionDescriptor {
    #[must_use]
    pub fn applies_to_role(self, role: GemTargetRole) -> bool {
        self.roles.contains(&role)
    }
}

/// Product-capability activation payload for one contribution instance.
///
/// "What did we build", distinct from host capabilities (ADR 0041's two
/// vocabularies). Generated glue constructs this from manifest-selected
/// capabilities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProductActivation {
    pub enabled: &'static [CapabilityId],
}

impl ProductActivation {
    #[must_use]
    pub fn contains(self, id: CapabilityId) -> bool {
        self.enabled.contains(&id)
    }
}
