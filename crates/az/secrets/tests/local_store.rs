use az_secret_value::Secret;
use az_secrets::{
    LocalSecretStore, MasterKeyProvider, ProvisionSecrets, ResolvedMasterKey, SecretBackend,
    SecretError, SecretRef,
};
use zeroize::Zeroizing;

struct FixedMasterKey([u8; 32]);

impl MasterKeyProvider for FixedMasterKey {
    fn load_or_create(&self) -> Result<ResolvedMasterKey, SecretError> {
        Ok(Secret::new(Zeroizing::new(self.0)))
    }
}

fn reference() -> SecretRef {
    SecretRef::parse("secret://project/auth-signing-key").expect("valid reference")
}

#[test]
fn local_store_seals_material_and_resolves_it() {
    let temp = tempfile::tempdir().expect("temporary store");
    let store = LocalSecretStore::open_in_directory(temp.path(), &FixedMasterKey([0x41; 32]))
        .expect("open encrypted store");

    store
        .provision(&reference(), b"machine-local-key")
        .expect("provision encrypted value");

    let stored = std::fs::read(store.location(&reference()).expect("value location"))
        .expect("read encrypted file");
    assert!(
        !stored
            .windows(b"machine-local-key".len())
            .any(|window| window == b"machine-local-key")
    );

    let resolved =
        futures::executor::block_on(store.resolve(&reference())).expect("resolve encrypted value");
    assert_eq!(resolved.as_bytes(), b"machine-local-key");
}

#[test]
fn wrong_key_tamper_and_plaintext_all_fail_closed() {
    let temp = tempfile::tempdir().expect("temporary store");
    let first = LocalSecretStore::open_in_directory(temp.path(), &FixedMasterKey([0x11; 32]))
        .expect("open first store");
    first
        .provision(&reference(), b"protected")
        .expect("provision value");

    let wrong_key = LocalSecretStore::open_in_directory(temp.path(), &FixedMasterKey([0x22; 32]))
        .expect("open store with wrong key");
    assert!(matches!(
        futures::executor::block_on(wrong_key.resolve(&reference())),
        Err(SecretError::DecryptionFailed)
    ));

    let path = first.location(&reference()).expect("value location");
    let mut ciphertext = std::fs::read(&path).expect("read ciphertext");
    let last = ciphertext.last_mut().expect("ciphertext byte");
    *last ^= 1;
    std::fs::write(&path, ciphertext).expect("tamper ciphertext");
    assert!(matches!(
        futures::executor::block_on(first.resolve(&reference())),
        Err(SecretError::DecryptionFailed)
    ));

    std::fs::write(&path, b"legacy plaintext").expect("write invalid legacy value");
    assert!(matches!(
        futures::executor::block_on(first.resolve(&reference())),
        Err(SecretError::UnsupportedCiphertext)
    ));
}

#[test]
fn empty_material_is_rejected_before_any_write() {
    let temp = tempfile::tempdir().expect("temporary store");
    let store = LocalSecretStore::open_in_directory(temp.path(), &FixedMasterKey([0x51; 32]))
        .expect("open encrypted store");

    assert!(matches!(
        store.provision(&reference(), &[]),
        Err(SecretError::EmptyMaterial)
    ));
    assert!(
        !store
            .location(&reference())
            .expect("value location")
            .exists()
    );
}

#[test]
fn missing_local_value_never_falls_back_to_an_implicit_environment_name() {
    let temp = tempfile::tempdir().expect("temporary store");
    let store = LocalSecretStore::open_in_directory(temp.path(), &FixedMasterKey([0x61; 32]))
        .expect("open encrypted store");

    let error = futures::executor::block_on(store.resolve(&reference())).unwrap_err();
    assert!(matches!(error, SecretError::Missing { .. }));
    assert!(!error.to_string().contains("AZOTH_SECRET_"));
}
