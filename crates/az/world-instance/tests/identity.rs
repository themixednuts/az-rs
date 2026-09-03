//! Worked rules for refined identities and the placement fence.
//!
//! Each fence case comes from ADR 0052: a placement fence is the exact logical
//! instance plus its generation, and a stale placement cannot publish
//! readiness, admit players, checkpoint, submit outcomes, or mutate lifecycle
//! state. Route publication joins that list because the accepted migration
//! requires a placement fence on ingress stage, publish, and withdraw.

use std::num::NonZeroU64;

use az_world_instance::{
    AdmissionTicketId, CheckpointId, FenceClaim, FencedEffect, IngressRouteId, PlacementFence,
    PlacementGeneration, PlayerPresenceId, ServerProcessId, WorldInstanceId,
    WorldInstanceIdentityKind, WorldInstanceOperationId, WorldInstancePlacementId, WorldTransferId,
};
use uuid::Uuid;

const EVERY_FENCED_EFFECT: [FencedEffect; 6] = [
    FencedEffect::PublishReadiness,
    FencedEffect::PublishIngressRoute,
    FencedEffect::AdmitPlayer,
    FencedEffect::CommitCheckpoint,
    FencedEffect::SubmitOutcome,
    FencedEffect::MutateLifecycleState,
];

fn instance(byte: u8) -> WorldInstanceId {
    WorldInstanceId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("instance")
}

fn generation(value: u64) -> PlacementGeneration {
    PlacementGeneration::try_from(value).expect("generation")
}

#[test]
fn the_current_fence_authorizes_every_fenced_effect_for_its_own_placement() {
    let world_instance = instance(0x11);
    let fence = PlacementFence::new(world_instance, PlacementGeneration::initial());

    for effect in EVERY_FENCED_EFFECT {
        fence
            .authorize(FenceClaim::new(
                world_instance,
                PlacementGeneration::initial(),
                effect,
            ))
            .unwrap_or_else(|error| panic!("current fence must authorize {effect}: {error}"));
    }
}

#[test]
fn a_superseded_generation_cannot_perform_any_fenced_effect() {
    let world_instance = instance(0x12);
    let superseded = PlacementFence::new(world_instance, PlacementGeneration::initial());
    let current = superseded.advanced().expect("recovery advances the fence");

    assert!(current.supersedes(superseded));
    assert!(!superseded.supersedes(current));

    for effect in EVERY_FENCED_EFFECT {
        let refusal = current
            .authorize(FenceClaim::new(
                world_instance,
                superseded.generation(),
                effect,
            ))
            .expect_err("a superseded placement must fail closed");

        assert_eq!(refusal.effect(), effect);
        assert_eq!(refusal.current(), current);
        assert_eq!(refusal.claim().generation(), superseded.generation());
    }
}

#[test]
fn a_future_generation_is_refused_because_it_was_never_granted() {
    let world_instance = instance(0x13);
    let current = PlacementFence::new(world_instance, PlacementGeneration::initial());
    let ungranted = current.advanced().expect("next generation").generation();

    let refusal = current
        .authorize(FenceClaim::new(
            world_instance,
            ungranted,
            FencedEffect::CommitCheckpoint,
        ))
        .expect_err("only the exact current generation is authorized");

    assert_eq!(refusal.current().generation(), current.generation());
}

#[test]
fn a_fence_never_authorizes_another_logical_instance() {
    let first = instance(0x31);
    let second = instance(0x32);
    let shared_generation = PlacementGeneration::initial();
    let fence = PlacementFence::new(first, shared_generation);

    let refusal = fence
        .authorize(FenceClaim::new(
            second,
            shared_generation,
            FencedEffect::AdmitPlayer,
        ))
        .expect_err("generations are scoped to one logical instance");

    assert_eq!(refusal.claim().world_instance(), second);
    assert_eq!(refusal.current().world_instance(), first);
    assert!(!PlacementFence::new(second, shared_generation).supersedes(fence));
}

#[test]
fn placement_replacement_advances_only_the_instance_scoped_fence() {
    let world_instance = instance(0x14);
    let first_placement =
        WorldInstancePlacementId::try_from_uuid(Uuid::from_bytes([0x21; 16])).expect("placement");
    let replacement =
        WorldInstancePlacementId::try_from_uuid(Uuid::from_bytes([0x22; 16])).expect("replacement");
    let unaffected = PlacementFence::new(instance(0x15), PlacementGeneration::initial());

    let first_fence = PlacementFence::new(world_instance, PlacementGeneration::initial());
    let replacement_fence = first_fence.advanced().expect("replacement fence");

    assert_ne!(first_placement, replacement);
    assert_eq!(replacement_fence.world_instance(), world_instance);
    assert_eq!(replacement_fence.generation(), generation(2));
    assert_eq!(unaffected.generation(), PlacementGeneration::initial());
    assert!(!replacement_fence.supersedes(unaffected));
}

#[test]
fn placement_generation_rejects_zero_and_overflow() {
    assert!(PlacementGeneration::try_from(0_u64).is_err());

    let maximum = PlacementFence::new(
        instance(0x16),
        PlacementGeneration::from_nonzero(NonZeroU64::new(u64::MAX).expect("maximum is nonzero")),
    );
    assert!(maximum.advanced().is_none());
}

#[test]
fn every_refined_identity_rejects_nil_and_reports_its_own_kind() {
    let nil = Uuid::nil();

    assert_eq!(
        WorldInstanceId::try_from_uuid(nil).expect_err("nil").kind(),
        WorldInstanceIdentityKind::Instance
    );
    assert_eq!(
        WorldInstancePlacementId::try_from_uuid(nil)
            .expect_err("nil")
            .kind(),
        WorldInstanceIdentityKind::Placement
    );
    assert_eq!(
        ServerProcessId::try_from_uuid(nil).expect_err("nil").kind(),
        WorldInstanceIdentityKind::ServerProcess
    );
    assert_eq!(
        IngressRouteId::try_from_uuid(nil).expect_err("nil").kind(),
        WorldInstanceIdentityKind::IngressRoute
    );
    assert_eq!(
        AdmissionTicketId::try_from_uuid(nil)
            .expect_err("nil")
            .kind(),
        WorldInstanceIdentityKind::AdmissionTicket
    );
    assert_eq!(
        PlayerPresenceId::try_from_uuid(nil)
            .expect_err("nil")
            .kind(),
        WorldInstanceIdentityKind::PlayerPresence
    );
    assert_eq!(
        WorldTransferId::try_from_uuid(nil).expect_err("nil").kind(),
        WorldInstanceIdentityKind::WorldTransfer
    );
    assert_eq!(
        CheckpointId::try_from_uuid(nil).expect_err("nil").kind(),
        WorldInstanceIdentityKind::Checkpoint
    );
    assert_eq!(
        WorldInstanceOperationId::try_from_uuid(nil)
            .expect_err("nil")
            .kind(),
        WorldInstanceIdentityKind::Operation
    );
}

#[test]
fn the_refined_identity_set_is_closed_and_distinctly_named() {
    let kinds = WorldInstanceIdentityKind::ALL;
    let labels = kinds
        .iter()
        .map(ToString::to_string)
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        kinds.len(),
        9,
        "ADR 0052 fixes nine refined lifecycle identities"
    );
    assert_eq!(
        labels.len(),
        kinds.len(),
        "identity labels must be distinct"
    );
    assert_eq!(
        kinds
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        kinds.len(),
        "identity kinds must be distinct"
    );
}
