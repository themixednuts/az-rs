use std::{fmt, path::Path, path::PathBuf};

use az_filesystem::{AzothDataHome, FileTransaction, FileWrite, safe_join};
use az_secret_value::Secret;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, Generate, KeyInit, Payload},
};
use zeroize::Zeroizing;

use crate::{
    BackendCapabilities, ResolvedSecret, SecretBackend, SecretError, SecretFuture, SecretRef,
};

const STORE_SCHEMA_VERSION: &str = "v1";
const SECRET_VALUE_FILE: &str = "value";
const CIPHERTEXT_MAGIC: &[u8; 4] = b"AZS1";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const KEYCHAIN_SERVICE: &str = "azoth.local-secret-master-key.v1";

/// Zeroizing master-key material returned only to the encrypted local store.
pub type ResolvedMasterKey = Secret<Zeroizing<[u8; 32]>>;

/// Narrow boundary from the encrypted local store to master-key custody.
pub trait MasterKeyProvider: Send + Sync {
    /// Loads or creates the master key for this already-bound data home.
    ///
    /// # Errors
    ///
    /// Returns a typed keychain or key-material failure.
    fn load_or_create(&self) -> Result<ResolvedMasterKey, SecretError>;
}

/// Operating-system keychain custody for one Azoth data home.
pub struct OsKeychainMasterKey {
    entry: keyring::Entry,
    creation_lock: PathBuf,
}

impl OsKeychainMasterKey {
    /// Binds a keychain entry to the normalized data-home identity.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::KeychainUnavailable`] when no platform keychain
    /// can securely store the master key.
    pub fn for_data_home(data_home: &AzothDataHome) -> Result<Self, SecretError> {
        let identity = blake3::hash(data_home.root().to_string_lossy().as_bytes());
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &identity.to_hex())
            .map_err(|_| SecretError::KeychainUnavailable)?;
        Ok(Self {
            entry,
            creation_lock: data_home.root().join("secret-master-key.lock"),
        })
    }

    fn decode_key(bytes: Vec<u8>) -> Result<ResolvedMasterKey, SecretError> {
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| SecretError::InvalidMasterKey)?;
        Ok(Secret::new(Zeroizing::new(key)))
    }
}

impl fmt::Debug for OsKeychainMasterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OsKeychainMasterKey")
            .finish_non_exhaustive()
    }
}

impl MasterKeyProvider for OsKeychainMasterKey {
    fn load_or_create(&self) -> Result<ResolvedMasterKey, SecretError> {
        if let Some(parent) = self.creation_lock.parent() {
            std::fs::create_dir_all(parent).map_err(|_| SecretError::KeychainUnavailable)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.creation_lock)
            .map_err(|_| SecretError::KeychainUnavailable)?;
        fs2::FileExt::lock_exclusive(&lock).map_err(|_| SecretError::KeychainUnavailable)?;
        match self.entry.get_secret() {
            Ok(bytes) => Self::decode_key(bytes),
            Err(keyring::Error::NoEntry) => {
                let generated = Key::generate();
                self.entry
                    .set_secret(generated.as_slice())
                    .map_err(|_| SecretError::KeychainUnavailable)?;
                let stored = self
                    .entry
                    .get_secret()
                    .map_err(|_| SecretError::KeychainUnavailable)?;
                Self::decode_key(stored)
            }
            Err(_) => Err(SecretError::KeychainUnavailable),
        }
    }
}

/// Provisioning capability implemented only by writable local stores.
pub trait ProvisionSecrets {
    /// Encrypts and atomically provisions one value.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, encryption, or transaction failure.
    fn provision(&self, secret: &SecretRef, material: &[u8]) -> Result<(), SecretError>;

    /// Returns the non-secret encrypted storage path for a reference.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::InvalidReference`] if the path cannot be joined
    /// beneath the store root.
    fn location(&self, secret: &SecretRef) -> Result<PathBuf, SecretError>;
}

/// Lore-instance-scoped encrypted local secret store.
pub struct LocalSecretStore {
    root: PathBuf,
    master_key: ResolvedMasterKey,
}

impl LocalSecretStore {
    /// Opens the active user's encrypted store for one project working tree.
    ///
    /// # Errors
    ///
    /// Returns a keychain or master-key failure before the store is usable.
    pub fn for_project(project_name: &str, project_root: &Path) -> Result<Self, SecretError> {
        let data_home = AzothDataHome::resolve();
        let root = data_home.project(project_name, project_root).secrets_dir();
        let key = OsKeychainMasterKey::for_data_home(&data_home)?;
        Self::open_in_directory(root, &key)
    }

    /// Opens an explicit root through an injected master-key custodian.
    ///
    /// # Errors
    ///
    /// Returns the provider's failure before any secret can be resolved.
    pub fn open_in_directory(
        root: impl Into<PathBuf>,
        provider: &dyn MasterKeyProvider,
    ) -> Result<Self, SecretError> {
        let master_key = provider.load_or_create()?;
        Ok(Self {
            root: root.into(),
            master_key,
        })
    }

    /// Returns the non-secret storage root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        let key = Key::from(**self.master_key.expose());
        ChaCha20Poly1305::new(&key)
    }

    fn seal(&self, secret: &SecretRef, material: &[u8]) -> Result<Vec<u8>, SecretError> {
        let nonce = Nonce::generate();
        let ciphertext = self
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: material,
                    aad: secret.as_str().as_bytes(),
                },
            )
            .map_err(|_| SecretError::EncryptionFailed)?;
        let mut encoded =
            Vec::with_capacity(CIPHERTEXT_MAGIC.len() + NONCE_BYTES + ciphertext.len());
        encoded.extend_from_slice(CIPHERTEXT_MAGIC);
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(encoded)
    }

    fn open_value(
        &self,
        secret: &SecretRef,
        encoded: &[u8],
    ) -> Result<ResolvedSecret, SecretError> {
        let minimum = CIPHERTEXT_MAGIC.len() + NONCE_BYTES + TAG_BYTES;
        if encoded.len() < minimum
            || encoded.get(..CIPHERTEXT_MAGIC.len()) != Some(CIPHERTEXT_MAGIC)
        {
            return Err(SecretError::UnsupportedCiphertext);
        }
        let nonce_start = CIPHERTEXT_MAGIC.len();
        let nonce_end = nonce_start + NONCE_BYTES;
        let nonce_bytes: [u8; NONCE_BYTES] = encoded[nonce_start..nonce_end]
            .try_into()
            .map_err(|_| SecretError::UnsupportedCiphertext)?;
        let nonce = Nonce::from(nonce_bytes);
        let plaintext = self
            .cipher()
            .decrypt(
                &nonce,
                Payload {
                    msg: &encoded[nonce_end..],
                    aad: secret.as_str().as_bytes(),
                },
            )
            .map_err(|_| SecretError::DecryptionFailed)?;
        if plaintext.is_empty() {
            return Err(SecretError::EmptyMaterial);
        }
        Ok(ResolvedSecret::from_bytes(plaintext))
    }

    fn value_path(&self, secret: &SecretRef) -> Result<PathBuf, SecretError> {
        safe_join(&self.root.join(STORE_SCHEMA_VERSION), secret.path())
            .map(|path| path.join(SECRET_VALUE_FILE))
            .map_err(|_| SecretError::InvalidReference)
    }

    fn resolve_sync(&self, secret: &SecretRef) -> Result<ResolvedSecret, SecretError> {
        let path = self.value_path(secret)?;
        let encoded = std::fs::read(&path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => SecretError::Missing {
                reference: secret.clone(),
            },
            _ => SecretError::Read {
                path: path.clone(),
                source,
            },
        })?;
        self.open_value(secret, &encoded)
    }
}

impl fmt::Debug for LocalSecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSecretStore")
            .field("root", &self.root)
            .field("master_key", &self.master_key)
            .finish()
    }
}

impl ProvisionSecrets for LocalSecretStore {
    fn provision(&self, secret: &SecretRef, material: &[u8]) -> Result<(), SecretError> {
        if material.is_empty() {
            return Err(SecretError::EmptyMaterial);
        }
        let path = self.value_path(secret)?;
        let encrypted = Zeroizing::new(self.seal(secret, material)?);
        let transaction = FileTransaction::new(self.root.join("transactions"));
        transaction
            .recover_pending()
            .map_err(SecretError::Transaction)?;
        transaction
            .commit([FileWrite::new(path, encrypted.to_vec())])
            .map_err(SecretError::Transaction)?;
        Ok(())
    }

    fn location(&self, secret: &SecretRef) -> Result<PathBuf, SecretError> {
        self.value_path(secret)
    }
}

impl SecretBackend for LocalSecretStore {
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a> {
        Box::pin(async move { self.resolve_sync(secret) })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::LOCAL_BINARY
    }
}
