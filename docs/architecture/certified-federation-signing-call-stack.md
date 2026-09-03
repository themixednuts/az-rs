# Certified Federation signing boundary

Linear ADR 0053 requires game servers to receive only their host-evidence signer.
They must never receive a federation root resolver, raw root key, provider key
handle, or role-erased signer that can be rebound to another purpose.

## Decision

Raw key bytes and public key-handle constructors are rejected because they let
the caller own custody. A role-erased `sign(bytes)` service is rejected because
one provider binding could then sign host evidence, authority receipts,
checkpoints, or roots.

`az-federation-crypto` instead owns `Signer<Role>`, `Signature<Role>`, and the
role-indexed `SigningPort<Role>`. A concrete provider adapter owns its handle
and credentials and implements only its configured role. `Signer::open` asks
that adapter to authenticate, verify permission and role binding, and report
the actual public-key identity before the application becomes ready. The
signer applies the public, versioned role domain before the provider call.

Role typing prevents accidental substitution inside an executable. It does not
turn possession of a key into federation authority. An independent verifier
must verify the signature and resolve a signed, current authorization binding
the `PublicKeyId` to the claimed role, scope, and authority epoch. That verifier
gate lands with the canonical grant and receipt slice; until then this crate
produces cryptographic claims, not accepted authority decisions.

`az-secrets` resolves the credential used by a concrete adapter to open its
provider-private handle. It does not own signing roles, public-key identity,
rotation, recovery, or audit.

The composition root names that key with a role-typed `SigningKeyHandle` and
hands it to `SigningKeyCustody<Role>`, the consumer-owned seam this crate owns.
Custody resolves the credential through whatever backend the deployment routes
to, binds the provider-private handle, and returns a port for that role only.
No credential, key material, provider client, or resolver crosses back, so a
caller can neither rebind the key to another role nor widen its own custody.
`open_game_server_signer` narrows that to `GameServerRole`, which only
`HostEvidence` implements: a game server's composition root cannot compile
against authority-receipt, history, or root-recovery custody, which is
ADR 0038's game-server rule made structural rather than procedural.

A key reference is a bounded printable-ASCII name, never material. Its
deployment meaning belongs to the custody adapter — ADR 0038 gives `az-secrets`
the `secret://` grammar and mount routing — so `SigningKeyRef` only bounds and
canonicalizes the name before it leaves the composition root.

## Public contract

```rust
pub trait SigningPort<R: SigningRole>: Send + Sync {
    fn open(&self) -> OpenSigningFuture<'_>;
    fn sign(&self, digest: ScopedDigest<R>) -> SigningFuture<'_>;
}

pub trait SigningKeyCustody<R: SigningRole>: Send + Sync {
    fn open_key<'a>(
        &'a self,
        handle: &'a SigningKeyHandle<R>,
    ) -> OpenKeyCustodyFuture<'a, R>;
}

pub async fn open_role_signer<R: SigningRole>(
    custody: &dyn SigningKeyCustody<R>,
    handle: &SigningKeyHandle<R>,
) -> Result<Signer<R>, SigningFailure>;

pub async fn open_game_server_signer<R: GameServerRole>(
    custody: &dyn SigningKeyCustody<R>,
    handle: &SigningKeyHandle<R>,
) -> Result<Signer<R>, SigningFailure>;

impl<R: SigningRole> Signer<R> {
    pub async fn open(
        provider: Arc<dyn SigningPort<R>>,
    ) -> Result<Self, SigningFailure>;

    pub async fn sign(
        &self,
        message: MessageDigest,
    ) -> Result<Signature<R>, SigningFailure>;
}
```

The public contract contains no handle, credential, provider SDK type, or
attacker-sized signature buffer. `ScopedDigest<Role>` prevents even a direct
port caller from supplying another role's digest. `RawSignature` is a closed
suite enum. A signature exposes its domain version, role, suite, and public-key
identity so a different implementation can select the same verification rules.
Each provider operation returns the actual signing-key identity with its raw
signature. The signer rejects a key that differs from startup attestation, so
provider alias rotation cannot silently mislabel a result.

Version one scopes a canonical 32-byte message digest with BLAKE3 derive-key:

```text
context = UTF8("azoth certified federation signature domain v1")
material = LE64(role_label_length) || role_label || message_digest_32
scoped_digest = BLAKE3 derive_key(context, material)
```

The exact context, role labels, construction function, and fixed vectors are
public compatibility commitments. Adding a suite or domain version appends a
new explicit variant; it does not reinterpret stored version-one signatures.

## Entrypoint-to-effect stack

```text
dedicated executable composition root
  1 the root names one key with SigningKeyHandle<ExactRole>
  2 open_role_signer calls SigningKeyCustody<ExactRole>::open_key
  3 custody resolves only this executable role's adapter credential
    through az-secrets, and the adapter privately binds its key handle
  4 Signer<ExactRole>::open calls SigningPort<ExactRole>::open
  5 adapter authenticates and attests permission, role binding, and key ID
  6 startup fails closed, or application receives Signer<ExactRole>

game or authority operation
  1 canonical protocol/domain code produces MessageDigest
  2 Signer<ExactRole>::sign derives the public versioned scoped digest
  3 SigningPort<ExactRole>::sign(scoped_digest) waits on the provider
  4 adapter returns actual key ID + signature or a typed provider failure
  5 signer rejects any key-ID change since startup attestation
  6 Signature<ExactRole> returns as an unaccepted cryptographic claim

independent acceptance path (mandatory later slice)
  1 verifier reconstructs the same versioned scoped digest
  2 verifier resolves public key material from PublicKeyId
  3 verifier verifies the suite-specific signature
  4 verifier checks signed role + scope + epoch authorization and revocation
  5 only then may domain code treat the claim as authorized evidence/receipt
```

The adapter owns the external waits and provider error translation. The
calling operation owns cancellation while waiting. No transaction or durable
authority result may commit before required signing succeeds. Retry policy
belongs to the durable operation that requested a signature, never to both the
signer and provider adapter.

## Vertical proof

| Behavior | Test |
|---|---|
| Role scoping, fixed digest vectors, key-identity change, typed failures | [`crates/az/federation-crypto/tests/role_scoped_signing.rs`](../../crates/az/federation-crypto/tests/role_scoped_signing.rs) |
| Custody resolves one named key, fails closed, and stays role-typed | [`crates/az/federation-crypto/tests/signing_key_custody.rs`](../../crates/az/federation-crypto/tests/signing_key_custody.rs) |
| Cryptographic verification and its role binding | [`crates/az/federation-crypto/tests/ed25519_verification.rs`](../../crates/az/federation-crypto/tests/ed25519_verification.rs), [`crates/az/federation-crypto/tests/verification_binding.rs`](../../crates/az/federation-crypto/tests/verification_binding.rs) |
| Signed history checkpoints and witness monitoring | [`crates/az/federation-crypto/tests/checkpoint_signing.rs`](../../crates/az/federation-crypto/tests/checkpoint_signing.rs), [`crates/az/federation-crypto/tests/witness_monitor.rs`](../../crates/az/federation-crypto/tests/witness_monitor.rs) |

The tests enter through `open_role_signer`, `Signer<Role>::open`, and `sign`,
cross deterministic external-provider fakes, and observe startup failure, the
exact fixed scoped digests, key identity, post-open key change, signature
metadata, and typed runtime failure. Separate host and authority port types
prove provider type substitution is rejected; adapter conformance must prove its
private handle is immutable or pinned. Rustdoc compile-fail examples in
[`crates/az/federation-crypto/src/key_handle.rs`](../../crates/az/federation-crypto/src/key_handle.rs)
and [`crates/az/federation-crypto/src/lib.rs`](../../crates/az/federation-crypto/src/lib.rs)
prove that role-indexed providers, custody, key handles, scoped digests, and
opened signers cannot be substituted, and that a game-server root cannot open a
non-host role. Signed role authorization in the canonical slice—not these
fakes—proves authority.
