# WorldInstance lifecycle ownership

`az-world-instance` is the provider-neutral lifecycle core. It owns stable
logical identity, replaceable placement identity, the placement fence, and the
effect ports that storage, provider, ingress, and distribution adapters
implement. Adapters depend on the core; the core depends on no game,
federation, provider, transport, or database implementation.

This page records the ownership that Wave 0 of the Dynamic WorldInstance
migration made executable. The decision itself is
[ADR 0052](https://linear.app/openworldserver/document/adr-0052-dynamic-worldinstance-hosting-350b01c3575f);
this page is the compiled shape of it. Nothing described here is wired into an
app, schedule, or generated target.

## Crate ownership

| Crate | Owns | Depends on |
| -- | -- | -- |
| `az-world-instance` | identity, launch spec, lifecycle, readiness, ingress, admission, transfer, checkpoint, release, outcome, closed errors, effect ports | generic engine value crates only |
| `az-world-instance-host` (not yet created) | local `InstanceHost`, child control protocol, OS containment, route attachment | `az-world-instance` |
| `az-world-instance-federation` (not yet created) | certified implementations of lifecycle-owned trust ports | `az-world-instance` + `az-federation` |
| provider integrations | external `PlacementProvider` / `IngressControl` / `ReleaseDistribution` adapters | `az-world-instance` |

`az-federation` already depends on `az-world-instance` for `PlacementFence`.
That direction is the contract: federation composes onto the lifecycle, and
the lifecycle never learns a federation type. ADR 0051 supplies the durable
subject, writer-fence, and outbox substrate that a storage adapter is built
from; the lifecycle consumes those semantics through `WorldInstanceStore`
rather than importing `az-durable` or copying its state machine.

## Refined identities

Nine identities are distinct nominal types. They are stamped from one closed
list in `src/identity.rs`, so a tenth cannot appear without joining
`WorldInstanceIdentityKind`, and `tests/ui/fail` proves no two of them are
interchangeable.

| Identity | Names |
| -- | -- |
| `WorldInstanceId` | one logical authoritative simulation occurrence |
| `WorldInstancePlacementId` | one replaceable execution attempt |
| `ServerProcessId` | one process owning a mutable simulation failure domain |
| `IngressRouteId` | one opaque virtual route |
| `AdmissionTicketId` | one single-use, client-key-bound ticket |
| `PlayerPresenceId` | one player's presence record |
| `WorldTransferId` | one durable transfer saga |
| `CheckpointId` | one committed placement-local checkpoint |
| `WorldInstanceOperationId` | one requested lifecycle change |

Every one of them rejects the nil UUID and reports which identity refused it.
Two monotonic counters stay separate from the identities and from each other:
`PlacementGeneration` fences execution, `WorldInstanceSpecRevision` orders
desired state.

## The placement fence

`PlacementFence` is the exact logical instance plus its current placement
generation. It authorizes a `FenceClaim` and returns a typed `StalePlacement`
otherwise. Three rules make it fail closed:

- a superseded generation is refused, which is what recovery relies on when it
  advances the generation for the same instance;
- an ungranted future generation is refused, because only the exact current
  generation is authorized; and
- another logical instance is refused even at the same generation number,
  because generations are instance-scoped and never order across instances.

`FencedEffect` is the closed set of effects a stale placement must never
perform. ADR 0052 names five — publish readiness, admit players, checkpoint,
submit outcomes, mutate lifecycle state. The accepted migration additionally
requires a placement fence on ingress stage, publish, and withdraw, so route
publication is the sixth member rather than an unfenced special case.

## Consumer-owned effect ports

Each port is declared by the lifecycle that consumes it, returns a named boxed
future (`PortFuture`) so composition roots can select adapters behind trait
objects, and answers with a closed lifecycle failure instead of a
provider-native error.

| Port | Module | Effect it fences |
| -- | -- | -- |
| `WorldInstanceStore` | `src/store.rs` | durable acceptance and fenced transitions |
| `PlacementProvider` | `src/provider.rs` | placement ensure, observe, drain, release |
| `IngressControl` | `src/ingress.rs` | route stage, publish, withdraw |
| `ReleaseDistribution` | `src/release.rs` | signed manifest and closure verification |

A method returning `PortFuture` is a remote effect. It leaves the control plane
and may never be reachable from a fixed-tick or replication schedule.

Fakes in `tests/ports.rs` implement all four the way an adapter would: they
hold the fence *they* know and recheck the caller's claim against it, which is
what makes a superseded placement unable to publish a route or commit a
transition. Writing those fakes is what surfaced that `Accepted`,
`WorldInstanceOperation`, and `WorldInstanceSnapshot` needed public
constructors — a store adapter cannot prove acceptance it is unable to mint.

## Structural guards

`tests/architecture.rs` scans this crate's manifest and sources. Each guard is
a pure function proven twice: against the real crate, where it must find
nothing, and against a fixture that deliberately violates it, where it must
fire.

| Guard | Rejects |
| -- | -- |
| `lifecycle_core_depends_only_on_allowlisted_value_crates` | any production dependency outside the allowlist, and any dependency that reverses the lifecycle's direction |
| `lifecycle_core_imports_only_allowlisted_namespaces` | any `use` path that does not resolve to this crate, `std`/`core`/`alloc`, an admitted engine namespace, or an allowlisted value crate |
| `lifecycle_core_declares_no_provider_or_federation_type` | a provider, federation, transport, database, or durable-substrate model declared in core |
| `lifecycle_core_declares_one_admission_presence_and_transfer_model` | a second admission ticket, presence authority, or transfer saga |
| `a_world_instance_never_becomes_a_cargo_target` | a generated build target, build script, non-`rlib` linkage, or code that reaches the build system |
| `no_remote_effect_is_reachable_from_a_fixed_tick_schedule` | a simulation-schedule symbol anywhere, or a remote effect declared outside the effect ports |

The dependency and import guards are stated positively: a crate nobody thought
to forbid still cannot enter the core, and widening the boundary has to be a
deliberate edit to the allowlist. Engine crates are admitted as a namespace
(`az_*`) and then filtered by an explicit rejection list, because lifecycle core
may consume generic engine value crates but never a federation, host, substrate,
or wire-record adapter.

Comment and documentation lines are excluded from the source scans so prose may
name the seams it describes. Trailing comments on a code line are still
scanned, which fails closed rather than open.

## What Wave 0 deliberately leaves open

- `WorldInstanceService` still owns a private in-memory reference store. Wave 1
  moves it behind `WorldInstanceStore`, adds `observe`, and lands the
  reconciler; the public `apply`/`get` contract does not change.
- Capacity storage is a fifth narrow port that arrives with Wave 3 resource
  scheduling. Admission, transfer, checkpoint, and outcome services arrive with
  Waves 5 to 7 and extend the sanctioned membership-model list rather than
  adding a second model.
- ADRs 0035, 0036, 0040, and 0041 carry the 2026-08-25 amendment that
  invalidates the wide-Rust DLL seam and records schema 9 static generated
  workspaces, so they agree with ADR 0052 that dynamic instances are runtime
  records and child processes. Neither names `InstanceHost` as a supervision
  owner or a host bundle entry; that is Wave 4 amendment surface, not a
  contradiction.
