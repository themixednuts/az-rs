use std::{fmt, str::Utf8Error};

use az_secret_value::Secret;
use zeroize::Zeroizing;

use crate::SecretError;

/// Zeroizing secret bytes with redacted diagnostics.
pub struct ResolvedSecret(Secret<Zeroizing<Vec<u8>>>);

impl ResolvedSecret {
    /// Takes ownership of resolved bytes.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Secret::new(Zeroizing::new(bytes)))
    }

    /// Explicitly borrows the resolved bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.expose().as_slice()
    }

    /// Interprets the bytes as UTF-8 without copying.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::InvalidUtf8`] when material is not text.
    pub fn as_utf8(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(self.as_bytes()).map_err(SecretError::InvalidUtf8)
    }

    /// Transfers bytes to another zeroizing owner.
    #[must_use]
    pub fn into_bytes(self) -> Zeroizing<Vec<u8>> {
        self.0.into_inner()
    }
}

impl fmt::Debug for ResolvedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl From<Utf8Error> for SecretError {
    fn from(error: Utf8Error) -> Self {
        Self::InvalidUtf8(error)
    }
}
