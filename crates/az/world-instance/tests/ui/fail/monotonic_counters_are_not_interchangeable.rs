//! A placement generation fences execution; a specification revision orders
//! desired state. Neither is the other, and neither is a bare integer.

use az_world_instance::{PlacementGeneration, WorldInstanceSpecRevision};

fn takes_generation(_: PlacementGeneration) {}
fn takes_revision(_: WorldInstanceSpecRevision) {}

fn main() {
    takes_generation(WorldInstanceSpecRevision::initial());
    takes_revision(PlacementGeneration::initial());
    takes_generation(1_u64);
}
