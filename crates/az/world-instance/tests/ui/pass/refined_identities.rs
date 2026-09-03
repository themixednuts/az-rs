//! The intended usage of the refined identities and the placement fence.

use az_world_instance::{
    AdmissionTicketId, CheckpointId, FenceClaim, FencedEffect, IngressRouteId, PlacementFence,
    PlacementGeneration, PlayerPresenceId, ServerProcessId, WorldInstanceId,
    WorldInstanceOperationId, WorldInstancePlacementId, WorldTransferId,
};

fn takes_instance(_: WorldInstanceId) {}
fn takes_placement(_: WorldInstancePlacementId) {}
fn takes_process(_: ServerProcessId) {}
fn takes_route(_: IngressRouteId) {}
fn takes_ticket(_: AdmissionTicketId) {}
fn takes_presence(_: PlayerPresenceId) {}
fn takes_transfer(_: WorldTransferId) {}
fn takes_checkpoint(_: CheckpointId) {}
fn takes_operation(_: WorldInstanceOperationId) {}

fn main() {
    let world_instance = WorldInstanceId::new();

    takes_instance(world_instance);
    takes_placement(WorldInstancePlacementId::new());
    takes_process(ServerProcessId::new());
    takes_route(IngressRouteId::new());
    takes_ticket(AdmissionTicketId::new());
    takes_presence(PlayerPresenceId::new());
    takes_transfer(WorldTransferId::new());
    takes_checkpoint(CheckpointId::new());
    takes_operation(WorldInstanceOperationId::new());

    let fence = PlacementFence::new(world_instance, PlacementGeneration::initial());
    fence
        .authorize(FenceClaim::new(
            fence.world_instance(),
            fence.generation(),
            FencedEffect::AdmitPlayer,
        ))
        .expect("the current fence authorizes its own placement");
}
