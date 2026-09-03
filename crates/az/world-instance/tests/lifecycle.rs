use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use az_world_instance::{
    AcceptanceDeadline, Accepted, ApplyWorldInstance, ApplyWorldInstanceError,
    ApplyWorldInstanceTarget, CanonicalApplyDigest, OperationActorId, WorldInstanceDefinition,
    WorldInstanceDesiredState, WorldInstanceId, WorldInstanceOperationId, WorldInstanceService,
    WorldInstanceSpecRevision,
};
use uuid::Uuid;

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn operation(byte: u8) -> WorldInstanceOperationId {
    WorldInstanceOperationId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("operation")
}

fn instance(byte: u8) -> WorldInstanceId {
    WorldInstanceId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("instance")
}

const fn digest(byte: u8) -> CanonicalApplyDigest {
    CanonicalApplyDigest::from_bytes([byte; 32])
}

const fn actor(byte: u8) -> OperationActorId {
    OperationActorId::from_bytes([byte; 32])
}

fn create_request(operation_byte: u8, instance_byte: u8, digest_byte: u8) -> ApplyWorldInstance {
    ApplyWorldInstance::new(
        operation(operation_byte),
        instance(instance_byte),
        ApplyWorldInstanceTarget::Create(WorldInstanceDefinition::new([0x51; 32])),
        WorldInstanceDesiredState::Running,
        actor(0x61),
        digest(digest_byte),
        AcceptanceDeadline::MAX,
    )
}

fn exact_request(
    operation_byte: u8,
    instance_byte: u8,
    expected: u64,
    desired: WorldInstanceDesiredState,
) -> ApplyWorldInstance {
    ApplyWorldInstance::new(
        operation(operation_byte),
        instance(instance_byte),
        ApplyWorldInstanceTarget::Exact(
            WorldInstanceSpecRevision::try_from(expected).expect("revision"),
        ),
        desired,
        actor(0x62),
        digest(operation_byte),
        AcceptanceDeadline::MAX,
    )
}

#[test]
fn create_then_exact_apply_advances_the_spec_revision() {
    let service = WorldInstanceService::in_memory();

    let created = block_on(service.apply(create_request(0x11, 0x21, 0x31))).expect("create");
    let drained = block_on(service.apply(exact_request(
        0x12,
        0x21,
        1,
        WorldInstanceDesiredState::Draining,
    )))
    .expect("drain");
    let snapshot = block_on(service.get(instance(0x21))).expect("snapshot");

    assert_eq!(created.value().spec_revision().get(), 1);
    assert_eq!(drained.value().spec_revision().get(), 2);
    assert_eq!(snapshot.spec_revision().get(), 2);
    assert_eq!(snapshot.desired(), WorldInstanceDesiredState::Draining);
    assert_eq!(
        snapshot.definition(),
        &WorldInstanceDefinition::new([0x51; 32])
    );
}

#[test]
fn exact_retry_returns_the_original_operation_and_changed_bytes_conflict() {
    let service = WorldInstanceService::in_memory();
    let request = create_request(0x13, 0x22, 0x32);

    let first = block_on(service.apply(request.clone())).expect("first");
    let retry = block_on(service.apply(request)).expect("retry");
    let conflict = block_on(service.apply(create_request(0x13, 0x22, 0x33)))
        .expect_err("changed canonical bytes conflict");

    assert_eq!(first, retry);
    assert!(matches!(
        conflict,
        ApplyWorldInstanceError::OperationPayloadConflict { .. }
    ));
    assert_eq!(
        block_on(service.get(instance(0x22)))
            .expect("snapshot")
            .spec_revision()
            .get(),
        1
    );
}

#[test]
fn create_existing_stale_revision_and_terminal_reopen_are_closed_errors() {
    let service = WorldInstanceService::in_memory();
    block_on(service.apply(create_request(0x14, 0x23, 0x34))).expect("create");

    assert!(matches!(
        block_on(service.apply(create_request(0x15, 0x23, 0x35))),
        Err(ApplyWorldInstanceError::AlreadyExists { .. })
    ));
    assert!(matches!(
        block_on(service.apply(exact_request(
            0x16,
            0x23,
            2,
            WorldInstanceDesiredState::Draining,
        ))),
        Err(ApplyWorldInstanceError::SpecRevisionConflict { .. })
    ));

    block_on(service.apply(exact_request(
        0x17,
        0x23,
        1,
        WorldInstanceDesiredState::Terminated,
    )))
    .expect("terminate");
    assert!(matches!(
        block_on(service.apply(exact_request(
            0x18,
            0x23,
            2,
            WorldInstanceDesiredState::Running,
        ))),
        Err(ApplyWorldInstanceError::TerminalInstance { .. })
    ));
}

#[test]
fn expired_new_operation_has_no_effect() {
    let service = WorldInstanceService::in_memory();
    let request =
        create_request(0x19, 0x24, 0x36).with_deadline(AcceptanceDeadline::from_unix_millis(0));

    assert!(matches!(
        block_on(service.apply(request)),
        Err(ApplyWorldInstanceError::AcceptanceDeadlineElapsed { .. })
    ));
    assert!(matches!(
        block_on(service.get(instance(0x24))),
        Err(az_world_instance::GetWorldInstanceError::NotFound { .. })
    ));
}

#[test]
fn concurrent_writers_at_one_revision_have_one_winner() {
    let service = WorldInstanceService::in_memory();
    block_on(service.apply(create_request(0x1a, 0x25, 0x37))).expect("create");

    let first = service.clone();
    let second = service;
    let first = std::thread::spawn(move || {
        block_on(first.apply(exact_request(
            0x1b,
            0x25,
            1,
            WorldInstanceDesiredState::Draining,
        )))
    });
    let second = std::thread::spawn(move || {
        block_on(second.apply(exact_request(
            0x1c,
            0x25,
            1,
            WorldInstanceDesiredState::Terminated,
        )))
    });
    let results: [Result<Accepted<_>, _>; 2] = [
        first.join().expect("first writer"),
        second.join().expect("second writer"),
    ];

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(ApplyWorldInstanceError::SpecRevisionConflict { .. })
            ))
            .count(),
        1
    );
}
