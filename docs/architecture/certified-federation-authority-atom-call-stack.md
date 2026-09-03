# Certified federation authority atom call stack

## Scope and constraints

This slice establishes the provider-neutral types and the first authoritative
mutation boundary required by Linear ADR 0053. A publisher defines the release,
ruleset, policy, and capability meaning. An operator may execute only under an
explicit, current binding. A verifier must be able to reproduce the exact
request bytes and authority facts used for a commit.

The boundary must provide:

- canonical bytes and a domain-separated request digest;
- non-nil operation and world-instance identities;
- an instance-scoped placement fence;
- exact retry deduplication and changed-payload conflict detection;
- one atomic state, receipt, dedupe, and transparency-outbox commit; and
- precise rejection of stale placement, grant, revocation, or state facts.

Cryptographic signing, durable storage, policy evaluation, and transparency
publication are later adapters. Their provider types must not enter these
domain contracts.

## Alternatives

### Selected: one consumer-owned authority-atom port

`AuthorityAtomPort::commit` accepts one fully bound `AuthorityAtomCommit` and
returns a domain result. The adapter owns serialization of competing writers,
idempotency, atomic persistence, and translation of storage unavailability.
This keeps all commit invariants at the effect boundary and lets production
storage replace the in-memory reference adapter without changing callers.

### Rejected: caller-coordinated repository calls

A caller could write state, dedupe, receipt, and outbox records through four
repositories. That makes partial commits representable, spreads retry and
transaction rules across every caller, and lets a stale authority check race
the write. It would only become preferable if those records deliberately had
independent authoritative lifecycles, which Linear ADR 0053 rejects.

### Rejected: a broad federation service

A service containing policy, signing, placement, storage, and publication
would hide ownership and couple provider choices to the domain. Separate typed
ports will be introduced only where subsequent behavior stacks expose a real
variable effect.

## Typed contracts and ownership

| Contract | Owner | Invariant |
|---|---|---|
| `WorldInstanceId`, `PlacementGeneration`, `PlacementFence` | [`crates/az/world-instance/src/identity.rs`](../../crates/az/world-instance/src/identity.rs) | A fence is the exact logical instance plus a nonzero generation. |
| `CertifiedExecutionBinding` | [`crates/az/federation/src/binding.rs`](../../crates/az/federation/src/binding.rs) | Federation, operator, host, placement, grant, release, ruleset, policy, and revocation facts are all mandatory. |
| `PortableMutationCommand` | [`crates/az/proto/federation/src/portable_mutation.rs`](../../crates/az/proto/federation/src/portable_mutation.rs) | The untrusted body is bounded before allocation and encodes to one canonical form. |
| `CanonicalRequestDigest` | [`crates/az/federation/src/canonical.rs`](../../crates/az/federation/src/canonical.rs) | The digest is computed over the complete canonical envelope with a versioned domain separator. |
| `AuthorityAtomCommit` | [`crates/az/federation/src/authority/atom.rs`](../../crates/az/federation/src/authority/atom.rs) | One operation carries its canonical digest, expected state version, complete binding, and result commitments. |
| `AuthorityAtomPort` | [`crates/az/federation/src/authority/atom.rs`](../../crates/az/federation/src/authority/atom.rs) | The consumer-facing seam owns one authoritative commit attempt. |
| `InMemoryAuthorityAtom` | [`crates/az/federation/src/authority/atom.rs`](../../crates/az/federation/src/authority/atom.rs) | Reference adapter proving transaction, fencing, and idempotency semantics; it is not the production durability claim. |
| `AuthorityOperationLookupPort` | [`crates/az/federation/src/authority/reconcile.rs`](../../crates/az/federation/src/authority/reconcile.rs) | An unknown outcome resolves against its original operation; no adapter answers for another atom. |
| `PlacementAuthorityPort` | [`crates/az/federation/src/authority/placement.rs`](../../crates/az/federation/src/authority/placement.rs) | Federation reads the live placement fence and drives no lifecycle transition. |

Protocol DTOs know canonical field tags and byte order. Domain code knows no
transport, database, cloud, key-provider, or game-specific types. A future
durable adapter may know its database transaction and row types, but must
translate them to `StorageFailure` at this seam.

## Entrypoint-to-effect stack

```text
decode_portable_mutation(&[u8])
  [az-proto-federation; untrusted synchronous boundary]
  -> validate magic, schema, kind, field count, tag order, lengths, and bounds
  -> construct refined identities and CertifiedExecutionBinding
  <- PortableMutationCommand | CanonicalEnvelopeError

encode_portable_mutation(&PortableMutationCommand)
  [az-proto-federation; hot-local deterministic work]
  -> emit canonical tagged fields in schema order
  -> BLAKE3 domain-separated canonical envelope bytes
  <- CanonicalEnvelope { bytes, CanonicalRequestDigest }

AuthorityAtomPort::commit(&AuthorityAtomCommit)
  [az-federation; asynchronous consumer-owned effect port]
  -> adapter serializes competing writes for one authority atom
  -> exact operation/digest retry check
  -> placement, grant, revocation, and expected-state checks
  -> stage state version, dedupe entry, receipt, and transparency outbox entry
  -> atomically publish the staged state
  <- Committed | Existing | OperationConflict | Rejected | StorageFailure

reconcile_uncertain_commit(&dyn AuthorityOperationLookupPort, &AuthorityAtomCommit)
  [az-federation; called only after an ambiguous commit outcome]
  -> load the accepted receipt for the original operation identity
  -> compare the stored canonical digest with the original request
  <- Accepted(receipt) | Conflicting | Unrecorded | StorageFailure
```

The reference adapter uses one mutex and holds no lock across an await point.
A production adapter may perform local-durable or remote storage work. It owns
the transaction deadline, cancellation, safe retry, and provider-error
translation. Callers may retry an unavailable commit with the same operation
ID and canonical digest; they must not generate a new operation ID for an
ambiguous outcome.

## Failure and convergence paths

- Exact duplicate: return `Existing` with the original receipt; do not mutate
  state or append another outbox entry.
- Same operation ID, different canonical digest: return `OperationConflict`;
  neither request may overwrite the other.
- Stale placement, grant revision, revocation cursor, or expected state:
  return the corresponding `CommitRefusal` before staging effects.
- Failure while staging any record: return `StorageFailure`; publish
  none of the staged state.
- Concurrent identical attempts: adapter serialization produces exactly one
  `Committed` result and all remaining attempts converge on `Existing`.
- Parse failure or unsupported schema/kind: no domain command or side effect is
  produced.

Cryptographic refusal and transparency-publication retry are intentionally not
collapsed into this stack. Signing will precede the commit with a typed
role-scoped signer; publication will consume the committed outbox after the
authoritative transaction. Neither can redefine a successful atom commit.

## Dependency boundaries

Federation consumes two accepted substrates and owns neither.

- **ADR 0051 durable work.** The atom's dedupe record is generic durable
  machinery; its meaning as an authority receipt is not. Federation reads an
  accepted operation through `AuthorityOperationLookupPort`, defined beside the
  atom that needs it, so no durable type, transaction, or cursor appears in a
  federation signature. A generic durable receipt never becomes an authority
  receipt.
- **ADR 0052 WorldInstance hosting.** `az-world-instance` owns placement,
  admission, presence, transfer, ingress, checkpoint, and destruction, and its
  `WorldInstanceService` is the only control surface for them. Federation reads
  exactly one fact — the live placement fence — through `PlacementAuthorityPort`
  and defines no second lifecycle, fence type, or admission model. The port is
  deliberately read-only; a mutation on it would move lifecycle authority into
  federation.

## Structural guards

[`crates/az/federation/src/architecture_tests.rs`](../../crates/az/federation/src/architecture_tests.rs)
scans the manifests and production sources of all three federation crates. Each
guard is paired with a fixture-violation test, so a green run means the rule
fires and this tree passes it.

| Guard | Rejects | Proven fireable by |
|---|---|---|
| `federation_crates_expose_no_provider_shaped_public_api` | Provider SDK, runtime, and database dependencies; a generic backend, service locator, command bus, or untyped policy bag | `provider_guard_fires_on_a_backend_dependency_and_a_locator_type` |
| `federation_declares_no_second_placement_admission_or_transfer_lifecycle` | Redeclaring any type `az-world-instance` exports, and naming the lifecycle control surface | `lifecycle_guard_fires_on_a_duplicate_ticket_type_and_a_lifecycle_call` |
| `federation_holds_no_direct_game_auth_or_storage_authority` | Any dependency that does not resolve under `crates/az/` or `crates/integrations/` and is not an allowlisted registry crate; engine crates carrying storage or secret authority; gem and legacy-format crates; direct reducer writer shapes | `storage_authority_guard_fires_on_a_direct_reducer_and_an_out_of_tree_dependency` |
| `federation_authority_waits_cannot_run_inside_a_simulation_schedule` | Simulation-host dependencies, blocking calls, schedule labels, and any synchronous effect-port method | `hot_path_guard_fires_on_a_blocking_call_and_a_synchronous_effect_port` |

Three coverage tests keep the scans honest:
`every_guard_reads_all_three_federation_crate_roots` fails if a path change
empties a source scan, `the_effect_port_scan_reads_every_federation_port` fails
if the effect-boundary parser stops seeing a declared port, and
`the_dependency_scan_resolves_workspace_aliases_to_engine_paths` fails if
workspace aliases stop resolving to real crate paths.

The dependency rule is stated positively: a production dependency either
resolves into the engine tree or is an allowlisted registry crate. Gems,
project crates, generated title bindings, and out-of-tree paths therefore fail
by construction, and no guard carries a game's name.

The lifecycle guard reads `az-world-instance`'s exported names as data, so it
keeps tracking ADR 0052 as that crate grows rather than aging into a stale list.

## Vertical proof

| Behavior | Test |
|---|---|
| Logical identity and placement fencing | [`crates/az/world-instance/tests/identity.rs`](../../crates/az/world-instance/tests/identity.rs) |
| Exact retry and changed-payload conflict | [`crates/az/federation/tests/canonical_atom.rs`](../../crates/az/federation/tests/canonical_atom.rs) |
| Precise stale-authority refusals | [`crates/az/federation/tests/canonical_atom.rs`](../../crates/az/federation/tests/canonical_atom.rs) |
| Unknown-outcome reconciliation and placement currency | [`crates/az/federation/tests/dependency_boundaries.rs`](../../crates/az/federation/tests/dependency_boundaries.rs) |
| Atomic failure at every staged write point | [`crates/az/federation/tests/canonical_atom.rs`](../../crates/az/federation/tests/canonical_atom.rs) |
| Concurrent same-operation convergence | [`crates/az/federation/tests/canonical_atom.rs`](../../crates/az/federation/tests/canonical_atom.rs) |
| Stable canonical bytes and digest vector | [`crates/az/proto/federation/tests/canonical_command.rs`](../../crates/az/proto/federation/tests/canonical_command.rs) |
| Malformed, reordered, unknown, oversized, nil, and unsupported inputs | [`crates/az/proto/federation/tests/canonical_command.rs`](../../crates/az/proto/federation/tests/canonical_command.rs) |
| Decoder panic resistance over truncations and bounded arbitrary inputs | [`crates/az/proto/federation/tests/canonical_command.rs`](../../crates/az/proto/federation/tests/canonical_command.rs) |

Production durability remains an explicit open proof obligation: a durable
adapter must run the same conformance cases across process restart and an
ambiguous post-commit transport failure.
