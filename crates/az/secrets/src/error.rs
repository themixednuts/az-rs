use std::{io, path::PathBuf, str::Utf8Error};

use crate::SecretRef;

/// Provider-neutral failure to configure, resolve, or provision a secret.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// Input was not a valid logical secret reference.
    #[error("value is not a valid `secret://` reference")]
    InvalidReference,
    /// Two backends claimed the same mount.
    #[error("secret mount `{mount}` is configured more than once")]
    DuplicateMount { mount: Box<str> },
    /// Two packages claimed the same backend id.
    #[error("secret backend `{backend}` is registered more than once")]
    DuplicateBackend { backend: Box<str> },
    /// Configuration selected an id that no linked package registered.
    #[error("secret backend `{backend}` is not registered in this host")]
    BackendNotRegistered { backend: Box<str> },
    /// No backend owns the reference's mount.
    #[error("secret mount `{mount}` is not configured")]
    MountNotConfigured { mount: Box<str> },
    /// A backend was called directly with a reference owned by another mount.
    #[error("secret backend for mount `{expected}` cannot resolve mount `{actual}`")]
    BackendMountMismatch {
        /// Mount bound at backend construction.
        expected: Box<str>,
        /// Mount present in the reference.
        actual: Box<str>,
    },
    /// The named value does not exist in the selected backend.
    #[error("secret `{reference}` is not provisioned")]
    Missing { reference: SecretRef },
    /// A selected value exists but has no bytes.
    #[error("secret material must not be empty")]
    EmptyMaterial,
    /// A caller requested text from binary material.
    #[error("secret material is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    /// Backend configuration cannot resolve values safely.
    #[error("secret backend configuration is invalid")]
    InvalidBackendConfiguration,
    /// Ambient provider authentication failed.
    #[error("secret backend authentication failed")]
    Authentication,
    /// Ambient identity authenticated but lacks permission for the value.
    #[error("secret backend denied access")]
    PermissionDenied,
    /// The selected backend is temporarily unavailable.
    #[error("secret backend is unavailable")]
    BackendUnavailable,
    /// A remote backend request failed at the network boundary.
    #[error("secret backend network request failed")]
    Network,
    /// The operating-system keychain could not open or persist the master key.
    #[error("operating-system keychain is unavailable for local secret storage")]
    KeychainUnavailable,
    /// Stored keychain material is not a valid master key.
    #[error("operating-system keychain returned invalid master-key material")]
    InvalidMasterKey,
    /// AEAD sealing failed without exposing material.
    #[error("encrypt local secret material")]
    EncryptionFailed,
    /// Stored material is not the version-one encrypted format.
    #[error("stored secret is not supported version-one ciphertext; re-provision it")]
    UnsupportedCiphertext,
    /// AEAD authentication or decryption failed.
    #[error("decrypt local secret material")]
    DecryptionFailed,
    /// Reading local encrypted material failed.
    #[error("read encrypted secret file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The recoverable encrypted-file transaction failed.
    #[error("encrypted secret transaction failed: {0}")]
    Transaction(#[source] az_filesystem::FileTransactionError),
}
