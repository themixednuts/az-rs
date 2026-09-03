# Engine secret-resolution call stack

Linear ADR 0038 moves secret resolution out of the auth gem and gives trusted hosts
and tools one provider-neutral, fail-closed boundary. Client and title-runtime
composition roots never link the resolver crate.

`az-project` owns the host-tool `[services.auth]` and `[secrets]` manifest
schema. It reuses runtime-neutral identity types from `az-gem-auth` and
`SecretRef`/mount types from `az-secrets`. Keeping the schema out of the shared
auth contract prevents a client that only needs credentials from acquiring a
resolver transitively.

## Resolve

```text
generated trusted-host composition root
  1 deserialize SecretRef; invalid scheme/path fails before startup wiring
  2 register only the linked backend factories by stable backend id
  3 add project -> local only when that mount was not explicitly rerouted
  4 construct SecretRouter from typed [secrets] mounts and the registry
  5 application asks SecretBackend::resolve(&SecretRef)
  6 router selects exactly the first-path-component mount; no fallback
  7 concrete backend performs its local, environment, or remote effect
  8 backend maps provider errors to SecretError and returns ResolvedSecret
  9 application explicitly exposes bytes and constructs its narrow capability
 10 any failure aborts host readiness
```

`SecretBackend` owns no retry or cache. A remote adapter may implement one
bounded provider retry policy; the startup caller owns its deadline and
cancellation. Resolution creates no durable transaction.

## Provision the encrypted local default

```text
azoth secret set
  1 CLI parses SecretRef and reads source bytes into a zeroizing owner
  2 mount configuration must select the local ProvisionSecrets capability
  3 LocalSecretStore serializes first creation per data home and loads or
    creates the master key in the OS keychain
  4 store generates a unique nonce and AEAD-seals bytes with reference as AAD
  5 FileTransaction atomically replaces the version-one ciphertext file
  6 CLI reports only the non-secret reference and storage location
```

The local store owns encryption and file recovery. The OS-keychain adapter owns
keychain errors. There is no plaintext or passphrase fallback. Cloud backends
do not implement `ProvisionSecrets`; their own deployment tooling owns writes.

## Batch and routing failures

The default `resolve_many` preserves input order and stops at the first error;
it returns no partial result. A backend overrides it only for a true atomic
vendor batch response. Missing mounts, ambiguous duplicate mounts, missing or
empty material, invalid text conversion, provider authentication, network,
keychain, decryption, and durable-write failures remain distinct typed errors.
