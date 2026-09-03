//! Fakes for every consumer-owned effect port.
//!
//! The point of these fakes is not behaviour: it is that the ports can be
//! implemented outside this crate at all. Each fake is written the way a
//! storage, provider, ingress, or distribution adapter would be — it receives a
//! fence, rechecks it, and answers with a closed lifecycle failure instead of a
//! provider-native error. If an adapter could not implement a port without
//! reaching into private lifecycle state, the seam would not be real.

use std::{
    collections::BTreeSet,
    future::ready,
    sync::Mutex,
    task::{Context, Poll, Waker},
};

use az_world_instance::{
    AcceptanceDeadline, Accepted, ApplyWorldInstance, ApplyWorldInstanceError,
    ApplyWorldInstanceTarget, ArtifactDigest, CanonicalApplyDigest, CommitTransitionError,
    CurrentRoute, DistributionError, EnsurePlacement, FencedEffect, FencedTransition,
    GetWorldInstanceError, IngressControl, IngressError, IngressRouteId, IngressRouteRequest,
    OpaqueRoute, OperationActorId, PlacementFence, PlacementGeneration, PlacementObservation,
    PlacementProvider, PlacementProviderError, PlacementRequest, PortFuture, ReleaseDistribution,
    ReleaseManifestDigest, ServerProcessId, StagedRoute, StoreError, VerifiedRelease,
    WorldInstanceDefinition, WorldInstanceDesiredState, WorldInstanceId, WorldInstanceOperation,
    WorldInstanceOperationId, WorldInstancePlacementId, WorldInstanceSnapshot,
    WorldInstanceSpecRevision, WorldInstanceStore,
};
use uuid::Uuid;

fn settled<T>(mut future: PortFuture<'_, T>) -> T {
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("port fakes settle without a runtime"),
    }
}

fn instance(byte: u8) -> WorldInstanceId {
    WorldInstanceId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("instance")
}

fn route(byte: u8) -> IngressRouteId {
    IngressRouteId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("route")
}

fn operation(byte: u8) -> WorldInstanceOperationId {
    WorldInstanceOperationId::try_from_uuid(Uuid::from_bytes([byte; 16])).expect("operation")
}

/// A provider adapter that only knows a fence and a process.
struct FakePlacementProvider {
    current: PlacementFence,
    process: ServerProcessId,
}

impl FakePlacementProvider {
    fn observe_fenced(
        &self,
        request: PlacementRequest,
        settled_observation: PlacementObservation,
    ) -> Result<PlacementObservation, PlacementProviderError> {
        self.current.authorize(request.claim())?;
        Ok(settled_observation)
    }
}

impl PlacementProvider for FakePlacementProvider {
    fn ensure(
        &self,
        request: EnsurePlacement,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>> {
        Box::pin(ready(self.observe_fenced(
            request.request(),
            PlacementObservation::Running {
                process: self.process,
            },
        )))
    }

    fn observe(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>> {
        Box::pin(ready(self.observe_fenced(
            request,
            PlacementObservation::Running {
                process: self.process,
            },
        )))
    }

    fn begin_drain(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>> {
        Box::pin(ready(
            self.observe_fenced(request, PlacementObservation::Draining),
        ))
    }

    fn release(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>> {
        Box::pin(ready(
            self.observe_fenced(request, PlacementObservation::Released),
        ))
    }
}

/// An ingress adapter that keeps the current fence and its staged routes.
///
/// It rechecks a publication claim against the fence *it* holds, never against
/// the fence the caller supplied, which is what makes a superseded placement
/// unable to publish.
struct FakeIngressControl {
    current: PlacementFence,
    staged: Mutex<BTreeSet<IngressRouteId>>,
}

impl FakeIngressControl {
    const fn new(current: PlacementFence) -> Self {
        Self {
            current,
            staged: Mutex::new(BTreeSet::new()),
        }
    }
}

impl IngressControl for FakeIngressControl {
    fn stage(
        &self,
        request: IngressRouteRequest,
    ) -> PortFuture<'_, Result<StagedRoute, IngressError>> {
        self.staged
            .lock()
            .expect("uncontended fake")
            .insert(request.route());
        Box::pin(ready(Ok(StagedRoute::new(OpaqueRoute::new(
            request.route(),
            [0x70; 32],
        )))))
    }

    fn publish(
        &self,
        request: IngressRouteRequest,
    ) -> PortFuture<'_, Result<CurrentRoute, IngressError>> {
        let result = self
            .current
            .authorize(request.publish_claim())
            .map_err(IngressError::from)
            .and_then(|()| {
                if self
                    .staged
                    .lock()
                    .expect("uncontended fake")
                    .contains(&request.route())
                {
                    Ok(CurrentRoute::new(OpaqueRoute::new(
                        request.route(),
                        [0x70; 32],
                    )))
                } else {
                    Err(IngressError::RouteNotStaged {
                        route: request.route(),
                    })
                }
            });
        Box::pin(ready(result))
    }

    fn withdraw(&self, request: IngressRouteRequest) -> PortFuture<'_, Result<(), IngressError>> {
        self.staged
            .lock()
            .expect("uncontended fake")
            .remove(&request.route());
        Box::pin(ready(Ok(())))
    }
}

/// A distribution adapter that verifies exactly one signed release.
struct FakeReleaseDistribution {
    known: ReleaseManifestDigest,
}

impl ReleaseDistribution for FakeReleaseDistribution {
    fn verify(
        &self,
        manifest: ReleaseManifestDigest,
    ) -> PortFuture<'_, Result<VerifiedRelease, DistributionError>> {
        Box::pin(ready(if manifest == self.known {
            Ok(VerifiedRelease::new(manifest, [0x80; 32]))
        } else {
            Err(DistributionError::UnknownRelease)
        }))
    }

    fn artifact_present(
        &self,
        _artifact: ArtifactDigest,
    ) -> PortFuture<'_, Result<bool, DistributionError>> {
        Box::pin(ready(Ok(true)))
    }
}

/// A durable owner that accepts one operation and one fenced transition.
struct FakeStore {
    current: PlacementFence,
    revision: WorldInstanceSpecRevision,
}

impl WorldInstanceStore for FakeStore {
    fn accept_apply(
        &self,
        request: ApplyWorldInstance,
    ) -> PortFuture<'_, Result<Accepted<WorldInstanceOperation>, ApplyWorldInstanceError>> {
        let accepted = Accepted::new(
            WorldInstanceOperation::new(&request, self.revision),
            1_700_000_000_000,
        );
        Box::pin(ready(Ok(accepted)))
    }

    fn commit_transition(
        &self,
        transition: FencedTransition,
    ) -> PortFuture<'_, Result<WorldInstanceSpecRevision, CommitTransitionError>> {
        let result = self
            .current
            .authorize(transition.claim())
            .map_err(CommitTransitionError::from)
            .and_then(|()| {
                if transition.expected_revision() == self.revision {
                    self.revision
                        .checked_next()
                        .ok_or(CommitTransitionError::Store(StoreError::Unavailable))
                } else {
                    Err(CommitTransitionError::SpecRevisionConflict {
                        world_instance: self.current.world_instance(),
                        expected: transition.expected_revision(),
                        current: self.revision,
                    })
                }
            });
        Box::pin(ready(result))
    }

    fn snapshot(
        &self,
        world_instance: WorldInstanceId,
    ) -> PortFuture<'_, Result<WorldInstanceSnapshot, GetWorldInstanceError>> {
        Box::pin(ready(Ok(WorldInstanceSnapshot::new(
            world_instance,
            WorldInstanceDefinition::new([0x51; 32]),
            self.revision,
            WorldInstanceDesiredState::Running,
            operation(0x41),
        ))))
    }
}

fn current_fence() -> PlacementFence {
    PlacementFence::new(instance(0x11), PlacementGeneration::initial())
}

#[test]
fn every_effect_port_is_object_safe_and_composable() {
    let current = current_fence();
    let provider = FakePlacementProvider {
        current,
        process: ServerProcessId::new(),
    };
    let ingress = FakeIngressControl::new(current);
    let distribution = FakeReleaseDistribution {
        known: ReleaseManifestDigest::from_bytes([0x90; 32]),
    };
    let store = FakeStore {
        current,
        revision: WorldInstanceSpecRevision::initial(),
    };

    // Composition roots select adapters behind trait objects.
    let provider: &dyn PlacementProvider = &provider;
    let ingress: &dyn IngressControl = &ingress;
    let distribution: &dyn ReleaseDistribution = &distribution;
    let store: &dyn WorldInstanceStore = &store;

    let placement = PlacementRequest::new(current, WorldInstancePlacementId::new());
    assert!(matches!(
        settled(provider.ensure(EnsurePlacement::new(placement, [0x51; 32]))),
        Ok(PlacementObservation::Running { .. })
    ));
    assert!(settled(ingress.stage(IngressRouteRequest::new(current, route(0x61)))).is_ok());
    assert!(settled(distribution.verify(ReleaseManifestDigest::from_bytes([0x90; 32]))).is_ok());
    assert!(settled(store.snapshot(instance(0x11))).is_ok());
}

#[test]
fn a_superseded_placement_is_refused_by_every_remote_effect() {
    let superseded = current_fence();
    let current = superseded.advanced().expect("recovery advances the fence");
    let provider = FakePlacementProvider {
        current,
        process: ServerProcessId::new(),
    };
    let ingress = FakeIngressControl::new(current);
    let store = FakeStore {
        current,
        revision: WorldInstanceSpecRevision::initial(),
    };

    let stale_placement = PlacementRequest::new(superseded, WorldInstancePlacementId::new());
    let stale_route = IngressRouteRequest::new(superseded, route(0x62));
    settled(ingress.stage(stale_route)).expect("staging is not publication");

    assert!(matches!(
        settled(provider.ensure(EnsurePlacement::new(stale_placement, [0x51; 32]))),
        Err(PlacementProviderError::StalePlacement(_))
    ));
    assert!(matches!(
        settled(provider.release(stale_placement)),
        Err(PlacementProviderError::StalePlacement(_))
    ));
    assert!(matches!(
        settled(ingress.publish(stale_route)),
        Err(IngressError::StalePlacement(_))
    ));
    assert!(matches!(
        settled(store.commit_transition(FencedTransition::new(
            superseded,
            FencedEffect::MutateLifecycleState,
            WorldInstanceSpecRevision::initial(),
            operation(0x42),
        ))),
        Err(CommitTransitionError::StalePlacement(_))
    ));
}

#[test]
fn a_route_is_published_only_after_it_is_staged() {
    let current = current_fence();
    let ingress = FakeIngressControl::new(current);
    let request = IngressRouteRequest::new(current, route(0x63));

    assert!(matches!(
        settled(ingress.publish(request)),
        Err(IngressError::RouteNotStaged { .. })
    ));

    let staged = settled(ingress.stage(request)).expect("stage");
    let published = settled(ingress.publish(request)).expect("publish");

    assert_eq!(staged.route(), published.route());
    assert!(settled(ingress.withdraw(request)).is_ok());
    assert!(matches!(
        settled(ingress.publish(request)),
        Err(IngressError::RouteNotStaged { .. })
    ));
}

#[test]
fn an_adapter_mints_acceptance_only_through_the_store_port() {
    let current = current_fence();
    let store = FakeStore {
        current,
        revision: WorldInstanceSpecRevision::initial(),
    };
    let request = ApplyWorldInstance::new(
        operation(0x43),
        current.world_instance(),
        ApplyWorldInstanceTarget::Create(WorldInstanceDefinition::new([0x51; 32])),
        WorldInstanceDesiredState::Running,
        OperationActorId::from_bytes([0x61; 32]),
        CanonicalApplyDigest::from_bytes([0x31; 32]),
        AcceptanceDeadline::MAX,
    );

    let accepted = settled(store.accept_apply(request)).expect("accept");
    let committed = settled(store.commit_transition(FencedTransition::new(
        current,
        FencedEffect::MutateLifecycleState,
        WorldInstanceSpecRevision::initial(),
        operation(0x43),
    )))
    .expect("commit");

    assert_eq!(accepted.value().id(), operation(0x43));
    assert_eq!(accepted.value().world_instance(), current.world_instance());
    assert_eq!(committed.get(), 2);
}
