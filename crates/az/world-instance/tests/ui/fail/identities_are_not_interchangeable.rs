//! Every refined identity is rotated into the next identity's position, so one
//! fixture proves all nine are nominally distinct rather than aliases of `Uuid`.

use az_world_instance::{
    AdmissionTicketId, CheckpointId, IngressRouteId, PlayerPresenceId, ServerProcessId,
    WorldInstanceId, WorldInstanceOperationId, WorldInstancePlacementId, WorldTransferId,
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
    takes_instance(WorldInstancePlacementId::new());
    takes_placement(ServerProcessId::new());
    takes_process(IngressRouteId::new());
    takes_route(AdmissionTicketId::new());
    takes_ticket(PlayerPresenceId::new());
    takes_presence(WorldTransferId::new());
    takes_transfer(CheckpointId::new());
    takes_checkpoint(WorldInstanceOperationId::new());
    takes_operation(WorldInstanceId::new());
}
