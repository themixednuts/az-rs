//! A raw UUID is not a lifecycle identity, and an identity's inner value is not
//! reachable for laundering one identity into another.

use az_world_instance::{WorldInstanceId, WorldInstanceOperationId};
use uuid::Uuid;

fn takes_instance(_: WorldInstanceId) {}

fn main() {
    takes_instance(Uuid::now_v7());

    let _: WorldInstanceId = Uuid::now_v7().into();

    let operation = WorldInstanceOperationId::new();
    let _ = WorldInstanceId(operation.as_uuid());
}
