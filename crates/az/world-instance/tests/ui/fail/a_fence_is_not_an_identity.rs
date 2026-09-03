//! A placement fence is the pair of a logical instance and its generation. It
//! is neither of its parts, its parts cannot be reached to forge a claim, and
//! an effect cannot be authorized without naming the claiming placement.

use az_world_instance::{FencedEffect, PlacementFence, PlacementGeneration, WorldInstanceId};

fn takes_instance(_: WorldInstanceId) {}
fn takes_generation(_: PlacementGeneration) {}

fn main() {
    let fence = PlacementFence::new(WorldInstanceId::new(), PlacementGeneration::initial());

    takes_instance(fence);
    takes_generation(fence);

    let _ = fence.authorize(FencedEffect::AdmitPlayer);
    let _ = fence.generation;
}
