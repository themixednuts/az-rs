//! Ids are distinct types: a gem id cannot be compared to a contribution id,
//! so passing one where another is expected cannot typecheck (ADR 0032).

use az_gem_contract::{ContributionId, GemId};

const GEM: GemId = GemId::new("azoth.vegetation");
const CONTRIBUTION: ContributionId = ContributionId::new("runtime");

fn main() {
    let _ = GEM == CONTRIBUTION;
}
